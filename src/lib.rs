//! An easy-to-use WebSocket server.
//!
//! To start a WebSocket listener, simply call [`launch()`], and use the
//! returned [`EventHub`] to react to client messages, connections, and disconnections.
//!
//! # Example
//!
//! A WebSocket echo server:
//!
//! ```no_run
//! use simple_websockets::{Event, Responder};
//! use std::collections::HashMap;
//!
//! fn main() {
//!     // listen for WebSockets on port 8080:
//!     let event_hub = simple_websockets::launch(8080)
//!         .expect("failed to listen on port 8080");
//!     // map between client ids and the client's `Responder`:
//!     let mut clients: HashMap<u64, Responder> = HashMap::new();
//!
//!     loop {
//!         match event_hub.poll_event() {
//!             Event::Connect(client_id, responder) => {
//!                 println!("A client connected with id #{}", client_id);
//!                 // add their Responder to our `clients` map:
//!                 clients.insert(client_id, responder);
//!             },
//!             Event::Disconnect(client_id) => {
//!                 println!("Client #{} disconnected.", client_id);
//!                 // remove the disconnected client from the clients map:
//!                 clients.remove(&client_id);
//!             },
//!             Event::Message(client_id, message) => {
//!                 println!("Received a message from client #{}: {:?}", client_id, message);
//!                 // retrieve this client's `Responder`:
//!                 let responder = clients.get(&client_id).unwrap();
//!                 // echo the message back:
//!                 responder.send(message);
//!             },
//!         }
//!     }
//! }
//! ```
use tokio::runtime::Runtime;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite, accept_async};
use futures_util::{StreamExt, SinkExt};

#[derive(Debug)]
pub enum Error {
    /// Returned by [`launch`] if the websocket listener thread failed to start
    FailedToStart,
}

/// An outgoing/incoming message to/from a websocket.
#[derive(Debug)]
pub enum Message {
    /// A text message
    Text(String),
    /// A binary message
    Binary(Vec<u8>),
}

impl Message {
    fn into_tungstenite(self) -> tungstenite::Message {
        match self {
            Self::Text(text) => tungstenite::Message::Text(text),
            Self::Binary(bytes) => tungstenite::Message::Binary(bytes),
        }
    }

    fn from_tungstenite(message: tungstenite::Message) -> Option<Self> {
        match message {
            tungstenite::Message::Binary(bytes) => Some(Self::Binary(bytes)),
            tungstenite::Message::Text(text) => Some(Self::Text(text)),
            _ => None,
        }
    }
}

enum ResponderCommand {
    Message(Message),
    CloseConnection,
}

/// Sends outgoing messages to a websocket.
/// Every connected websocket client has a corresponding `Responder`.
///
/// `Responder`s can be safely cloned and sent across threads, to be used in a
/// multi-producer single-consumer paradigm.
///
/// If a Reponder is dropped while its client is still connected, the connection
/// will be automatically closed. If there are multiple clones of a Responder,
/// The client will not be disconnected until the last Responder is dropped.
#[derive(Debug, Clone)]
pub struct Responder {
    tx: flume::Sender<ResponderCommand>,
    client_id: u64,
}

impl Responder {
    fn new(tx: flume::Sender<ResponderCommand>, client_id: u64) -> Self {
        Self {
            tx,
            client_id,
        }
    }

    /// Sends a message to the client represented by this `Responder`.
    ///
    /// Returns true if the message was sent, or false if it wasn't
    /// sent (because the client is disconnected).
    ///
    /// Note that this *doesn't* need a mutable reference to `self`.
    pub fn send(&self, message: Message) -> bool {
        self.tx.send(ResponderCommand::Message(message))
            .is_ok()
    }

    /// Closes this client's connection.
    ///
    /// Note that this *doesn't* need a mutable reference to `self`.
    pub fn close(&self) {
        let _ = self.tx.send(ResponderCommand::CloseConnection);
    }

    /// The id of the client that this `Responder` is connected to.
    pub fn client_id(&self) -> u64 {
        self.client_id
    }
}

/// An incoming event from a client.
/// This can be an incoming message, a new client connection, or a disconnection.
#[derive(Debug)]
pub enum Event {
    /// A new client has connected.
    Connect(
        /// id of the client who connected
        u64,
        /// [`Responder`] used to send messages back to this client
        Responder,
    ),

    /// A client has disconnected.
    Disconnect(
        /// id of the client who disconnected
        u64,
    ),

    /// An incoming message from a client.
    Message(
        /// id of the client who sent the message
        u64,
        /// the message
        Message,
    ),
}

/// A queue of incoming events from clients.
///
/// The `EventHub` is the centerpiece of this library, and it is where all
/// messages, connections, and disconnections are received.
#[derive(Debug)]
pub struct EventHub {
    rx: flume::Receiver<Event>,
}

impl EventHub {
    fn new(rx: flume::Receiver<Event>) -> Self {
        Self {
            rx,
        }
    }

    /// Clears the event queue and returns all the events that were in the queue.
    pub fn drain(&self) -> Vec<Event> {
        if self.rx.is_disconnected() && self.rx.is_empty() {
            panic!("EventHub channel disconnected. Panicking because Websocket listener thread was killed.");
        }

        self.rx.drain().collect()
    }

    /// Returns the next event, or None if the queue is empty.
    pub fn next_event(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Returns the next event, blocking if the queue is empty.
    pub fn poll_event(&self) -> Event {
        self.rx.recv().unwrap()
    }

    /// Async version of [`poll_event`](Self::poll_event)
    pub async fn poll_async(&self) -> Event {
        self.rx.recv_async().await
            .expect("Parent thread is dead")
    }

    /// Returns true if there are currently no events in the queue.
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

/// Start listening for websocket connections on `port`.
/// On success, returns an [`EventHub`] for receiving messages and
/// connection/disconnection notifications.
pub fn launch(port: u16) -> Result<EventHub, Error> {
    let (tx, rx) = flume::unbounded();

    std::thread::Builder::new()
        .name("Websocket listener".to_string())
        .spawn(move || {
            start_runtime(tx, port).unwrap();
        }).map_err(|_| Error::FailedToStart)?;

    Ok(EventHub::new(rx))
}

fn start_runtime(event_tx: flume::Sender<Event>, port: u16) -> Result<(), Error> {
    Runtime::new()
        .map_err(|_| Error::FailedToStart)?
        .block_on(async {
            let address = format!("0.0.0.0:{}", port);
            let listener = TcpListener::bind(&address).await
                .map_err(|_| Error::FailedToStart)?;

            let mut current_id: u64 = 0;
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        tokio::spawn(handle_connection(stream, event_tx.clone(), current_id));
                        current_id = current_id.wrapping_add(1);
                    },
                    _ => {},
                }
            }
        })
}

async fn handle_connection(stream: TcpStream, event_tx: flume::Sender<Event>, id: u64) {
    let ws_stream = match accept_async(stream).await {
        Ok(s) => s,
        Err(_) => return,
    };

    let (mut outgoing, mut incoming) = ws_stream.split();

    // channel for the `Responder` to send things to this websocket
    let (resp_tx, resp_rx) = flume::unbounded();

    event_tx.send(Event::Connect(id, Responder::new(resp_tx, id)))
        .expect("Parent thread is dead");

    // future that waits for commands from the `Responder`
    let responder_events = async move {
        while let Ok(event) = resp_rx.recv_async().await {
            match event {
                ResponderCommand::Message(message) => {
                    if let Err(_) = outgoing.send(message.into_tungstenite()).await {
                        let _ = outgoing.close().await;
                        return Ok(());
                    }
                },
                ResponderCommand::CloseConnection => {
                    let _ = outgoing.close().await;
                    return Ok(());
                },
            }
        }

        // Disconnect if the `Responder` was dropped without explicitly disconnecting
        let _ = outgoing.close().await;

        // this future always returns Ok, so that it wont stop the try_join
        Result::<(), ()>::Ok(())
    };

    let event_tx2 = event_tx.clone();
    //future that forwards messages received from the websocket to the event channel
    let events = async move {
        while let Some(message) = incoming.next().await {
            if let Ok(tungstenite_msg) = message {
                if let Some(msg) = Message::from_tungstenite(tungstenite_msg) {
                    event_tx2.send(Event::Message(id, msg))
                        .expect("Parent thread is dead");
                }
            }
        }

        // stop the try_join once the websocket is closed and all pending incoming
        // messages have been sent to the event channel.
        // stopping the try_join causes responder_events to be closed too so that the
        // `Receiver` cant send any more messages.
        Result::<(), ()>::Err(())
    };

    // use try_join so that when `events` returns Err (the websocket closes), responder_events will be stopped too
    let _ = futures_util::try_join!(responder_events, events);

    event_tx.send(Event::Disconnect(id))
        .expect("Parent thread is dead");
}
