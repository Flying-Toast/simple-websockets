# simple-websockets

An easy-to-use WebSocket server.

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Docs](https://docs.rs/simple-websockets/badge.svg)](https://docs.rs/simple-websockets)
[![Crates.io](https://img.shields.io/crates/v/simple-websockets.svg)](https://crates.io/crates/simple-websockets)

# Example

Echo server:

```rust
use simple_websockets::{Event, Responder};
use std::collections::HashMap;

fn main() {
    // listen for WebSockets on port 8080:
    let event_hub = simple_websockets::launch(8080)
        .expect("failed to listen on port 8080");
    // map between client ids and the client's `Responder`:
    let mut clients: HashMap<u64, Responder> = HashMap::new();

    loop {
        match event_hub.poll_event() {
            Event::Connect(client_id, responder) => {
                println!("A client connected with id #{}", client_id);
                // add their Responder to our `clients` map:
                clients.insert(client_id, responder);
            },
            Event::Disconnect(client_id) => {
                println!("Client #{} disconnected.", client_id);
                // remove the disconnected client from the clients map:
                clients.remove(&client_id);
            },
            Event::Message(client_id, message) => {
                println!("Received a message from client #{}: {:?}", client_id, message);
                // retrieve this client's `Responder`:
                let responder = clients.get(&client_id).unwrap();
                // echo the message back:
                responder.send(message);
            },
        }
    }
}
```
