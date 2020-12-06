use tokio::runtime::Runtime;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite, accept_async};
use futures_util::{StreamExt, SinkExt};

#[derive(Debug)]
pub enum Error {
    ClientNotConnected,
    FailedToStart,
}

#[derive(Debug)]
pub enum Message {
    Text(String),
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

/// For sending messages to the a websocket
#[derive(Debug)]
pub struct Responder {
    tx: flume::Sender<ResponderCommand>,
}

impl Responder {
    fn new(tx: flume::Sender<ResponderCommand>) -> Self {
        Self {
            tx,
        }
    }

    pub fn send(&self, message: Message) -> Result<(), Error> {
        self.tx.send(ResponderCommand::Message(message))
            .map_err(|_| Error::ClientNotConnected)
    }

    pub fn close(&self) -> Result<(), Error> {
        self.tx.send(ResponderCommand::CloseConnection)
            .map_err(|_| Error::ClientNotConnected)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ClientID(u32);

#[derive(Debug)]
pub enum Event {
    Connect(ClientID, Responder),
    Disconnect(ClientID),
    Message(ClientID, Message),
}

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

    pub fn drain(&mut self) -> Vec<Event> {
        if self.rx.is_disconnected() && self.rx.is_empty() {
            panic!("EventHub channel disconnected");
        }

        self.rx.drain().collect()
    }
}

pub fn launch(port: u16) -> Result<EventHub, Error> {
    let (tx, rx) = flume::unbounded();

    std::thread::Builder::new()
        .name("network".to_string())
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

            let mut current_id: u32 = 0;
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        tokio::spawn(handle_connection(stream, event_tx.clone(), ClientID(current_id)));
                        current_id = current_id.wrapping_add(1);
                    },
                    Err(e) => eprintln!("Error accepting websocket connection: {:?}", e),
                }
            }
        })
}

async fn handle_connection(stream: TcpStream, event_tx: flume::Sender<Event>, id: ClientID) {
    let ws_stream = match accept_async(stream).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Websocket handshake failed: {:?}", e);
            return;
        },
    };

    let (mut outgoing, mut incoming) = ws_stream.split();

    // channel for the `Responder` to send things to this websocket
    let (resp_tx, resp_rx) = flume::unbounded();

    event_tx.send(Event::Connect(id, Responder::new(resp_tx)))
        .expect("Event channel disconnected");

    // future that waits for commands from the `Responder`
    let responder_events = async move {
        while let Ok(event) = resp_rx.recv_async().await {
            match event {
                ResponderCommand::Message(message) => {
                    if let Err(e) = outgoing.send(message.into_tungstenite()).await {
                        eprintln!("Error sending to client ({:?}): {} - disconnecting", id, e);
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

        eprintln!("Client's ({:?}) `Responder` was dropped without explicitly disconnecting - disconnecting now", id);
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
                        .expect("Server channel disconnected");
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
        .expect("Server channel disconnected");
}
