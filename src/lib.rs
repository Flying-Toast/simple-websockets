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
    ///
    /// The first field is a `u64`, which is a unique id of the connected client.
    /// The second field is a [`Responder`] which is used to send messages to this client.
    Connect(u64, Responder),
    /// A client has disconnected.
    ///
    /// The `u64` field is the id of the client that disconnected.
    Disconnect(u64),
    /// An incoming message from a client.
    ///
    /// The `u64` field is the id of the client who sent this message.
    /// The second field is the actual [`Message`].
    Message(u64, Message),
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
    pub fn drain(&mut self) -> Vec<Event> {
        if self.rx.is_disconnected() && self.rx.is_empty() {
            panic!("EventHub channel disconnected. Panicking because Websocket listener thread was killed.");
        }

        self.rx.drain().collect()
    }

    /// Returns true if there are currently no events in the queue.
    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

/// Start listening for websocket connections.
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
