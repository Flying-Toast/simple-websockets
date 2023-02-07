#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use simple_websockets;
    use url::Url;

    #[test]
    fn connect_disconnect_test() {
        // Start a server
        let (websocket_event_hub, server_endpoint) = start_websocket_and_get_server_endpoint();
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert!(websocket_event_hub.is_empty());

        // Connect and disconnect some clients and assert on server events
        let (mut client_0, _response_0) =
            tungstenite::connect(&server_endpoint).expect("Can't connect");
        std::thread::sleep(std::time::Duration::from_millis(500)); // Ensure event is actually triggered. The longer the wait, the slower test. The short the wait the higher risk of getting an unstable test. Optimal solution would be for
        assert_connect_event(websocket_event_hub.poll_event(), 0);
        assert!(websocket_event_hub.is_empty());

        let (mut client_1, _response_1) =
            tungstenite::connect(&server_endpoint).expect("Can't connect");
        let (mut _client_2, _response_2) =
            tungstenite::connect(&server_endpoint).expect("Can't connect");

        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!websocket_event_hub.is_empty());
        assert_connect_event(websocket_event_hub.poll_event(), 1);
        assert!(!websocket_event_hub.is_empty());
        assert_connect_event(websocket_event_hub.poll_event(), 2);
        assert!(websocket_event_hub.is_empty());

        client_1
            .close(None)
            .expect("Expected no panic from close call");
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!websocket_event_hub.is_empty());

        assert_disconnect_event(websocket_event_hub.poll_event(), 1);
        assert!(websocket_event_hub.is_empty());

        client_0
            .close(None)
            .expect("Expected no panic from close call");
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!websocket_event_hub.is_empty());

        assert_disconnect_event(websocket_event_hub.poll_event(), 0);
        assert!(websocket_event_hub.is_empty());
    }

    #[test]
    fn receive_text_message_test() {
        // Start a server
        let (websocket_event_hub, server_endpoint) = start_websocket_and_get_server_endpoint();
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert!(websocket_event_hub.is_empty());

        // Connect some clients and send from the middle one to ensure no bug exists that always returns first or last client id for a received message
        let (_client_0, _) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 0);

        let (mut client_1, _) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 1);

        let (mut client_2, _) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 2);

        assert!(websocket_event_hub.is_empty());

        client_1
            .write_message(tungstenite::Message::Text(String::from(
                "Hello from client 1!",
            )))
            .expect("Error sending text message");
        assert_text_message_event(websocket_event_hub.poll_event(), 1, "Hello from client 1!");
        assert!(websocket_event_hub.is_empty());

        client_1
            .write_message(tungstenite::Message::Text(String::from(
                "Hello from client 1 again!",
            )))
            .expect("Error sending text message");
        assert_text_message_event(
            websocket_event_hub.poll_event(),
            1,
            "Hello from client 1 again!",
        );

        client_2
            .write_message(tungstenite::Message::Text(String::from(
                "Hello from client 2!",
            )))
            .expect("Error sending text message");
        assert_text_message_event(websocket_event_hub.poll_event(), 2, "Hello from client 2!");
        assert!(websocket_event_hub.is_empty());
    }

    #[test]
    fn receive_binary_message_test() {
        // Start a server
        let (websocket_event_hub, server_endpoint) = start_websocket_and_get_server_endpoint();
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert!(websocket_event_hub.is_empty());

        // Connect some clients and send from the middle one to ensure no bug exists that always returns first or last client id for a received message
        let (_client_0, _) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 0);

        let (mut client_1, _) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 1);

        let (mut client_2, _) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 2);

        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(websocket_event_hub.is_empty());

        client_1
            .write_message(tungstenite::Message::Binary(vec![1, 2, 3]))
            .expect("Error sending text message");
        assert_binary_message_event(websocket_event_hub.poll_event(), 1, vec![1, 2, 3]);
        assert!(websocket_event_hub.is_empty());

        client_1
            .write_message(tungstenite::Message::Binary(vec![]))
            .expect("Error sending text message");
        assert_binary_message_event(websocket_event_hub.poll_event(), 1, vec![]);

        client_2
            .write_message(tungstenite::Message::Binary(vec![4, 5, 6]))
            .expect("Error sending text message");
        assert_binary_message_event(websocket_event_hub.poll_event(), 2, vec![4, 5, 6]);
        assert!(websocket_event_hub.is_empty());
    }

    #[test]
    fn send_text_message_test() {
        // Start a server
        let (websocket_event_hub, server_endpoint) = start_websocket_and_get_server_endpoint();
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert!(websocket_event_hub.is_empty());

        // Connect some clients and send from the middle one to ensure no bug exists that always returns first or last client id for a received message
        let (_c0, _r0) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 0);

        let (mut client_1, _r1) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        let (_, responder_1) = assert_connect_event(websocket_event_hub.poll_event(), 1);

        let (mut client_2, _r2) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        let (_, responder_2) = assert_connect_event(websocket_event_hub.poll_event(), 2);

        assert!(websocket_event_hub.is_empty());

        responder_1.send(simple_websockets::Message::Text(
            "Hello client 1!".to_string(),
        ));
        responder_2.send(simple_websockets::Message::Text(
            "Hello client 2!".to_string(),
        ));

        match client_1.read_message().unwrap() {
            tungstenite::Message::Text(text) => {
                assert_eq!("Hello client 1!", text);
            }
            _ => panic!("Unexpected type!"),
        }

        match client_2.read_message().unwrap() {
            tungstenite::Message::Text(text) => {
                assert_eq!("Hello client 2!", text);
            }
            _ => panic!("Unexpected type!"),
        }
    }

    #[test]
    fn send_binary_message_test() {
        // Start a server
        let (websocket_event_hub, server_endpoint) = start_websocket_and_get_server_endpoint();
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert!(websocket_event_hub.is_empty());

        // Connect some clients and send from the middle one to ensure no bug exists that always returns first or last client id for a received message
        let (_c0, _r0) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        assert_connect_event(websocket_event_hub.poll_event(), 0);

        let (mut client_1, _r1) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        let (_, responder_1) = assert_connect_event(websocket_event_hub.poll_event(), 1);

        let (mut client_2, _r2) = tungstenite::connect(&server_endpoint).expect("Can't connect");
        let (_, responder_2) = assert_connect_event(websocket_event_hub.poll_event(), 2);

        assert!(websocket_event_hub.is_empty());

        responder_1.send(simple_websockets::Message::Binary(vec![1, 2, 3]));
        responder_2.send(simple_websockets::Message::Binary(vec![4, 5, 6]));

        match client_1.read_message().unwrap() {
            tungstenite::Message::Binary(bytes) => {
                assert_eq!(3, bytes.len());
                assert_eq!(1, bytes[0]);
                assert_eq!(2, bytes[1]);
                assert_eq!(3, bytes[2]);
            }
            _ => panic!("Unexpected type!"),
        }

        match client_2.read_message().unwrap() {
            tungstenite::Message::Binary(bytes) => {
                assert_eq!(3, bytes.len());
                assert_eq!(4, bytes[0]);
                assert_eq!(5, bytes[1]);
                assert_eq!(6, bytes[2]);
            }
            _ => panic!("Unexpected type!"),
        }
    }

    #[test]
    fn launch_bind_failed_expect_error_received_test() {
        // Occupy a random port
        let listener = TcpListener::bind(format!("0.0.0.0:0")).unwrap();
        let taken_port = listener.local_addr().unwrap().port();

        // Call launch with the port that we now know is taken - expect an error
        simple_websockets::launch(taken_port).expect_err("Expected error binding to same port");
    }

    fn start_websocket_and_get_server_endpoint() -> (simple_websockets::EventHub, Url) {
        let listener = TcpListener::bind(format!("0.0.0.0:0")).unwrap();
        let port = listener.local_addr().unwrap().port();

        let websocket_event_hub = simple_websockets::launch_from_listener(listener)
            .expect(format!("failed to listen on websocket port unspecified port").as_str());
        let server_endpoint = Url::parse(format!("ws://127.0.0.1:{port}").as_str()).unwrap();
        return (websocket_event_hub, server_endpoint);
    }

    fn assert_connect_event(
        e: simple_websockets::Event,
        expected_client_id: u64,
    ) -> (u64, simple_websockets::Responder) {
        match e {
            simple_websockets::Event::Connect(client_id, responder) => {
                assert_eq!(expected_client_id, client_id);
                return (client_id, responder);
            }
            simple_websockets::Event::Disconnect(_client_id) => {
                panic!("Disconnect event received but expected connect event")
            }
            simple_websockets::Event::Message(_client_id, _message) => {
                panic!("Message event received but expected connect event")
            }
        }
    }

    fn assert_disconnect_event(e: simple_websockets::Event, expected_client_id: u64) {
        match e {
            simple_websockets::Event::Connect(_client_id, _response) => {
                panic!("Connect event received but expected disconnect event")
            }
            simple_websockets::Event::Disconnect(client_id) => {
                assert_eq!(expected_client_id, client_id)
            }
            simple_websockets::Event::Message(_client_id, _message) => {
                panic!("Message event received but expected disconnect event")
            }
        }
    }

    fn assert_text_message_event(
        e: simple_websockets::Event,
        expected_client_id: u64,
        expected_text_content: &str,
    ) {
        match e {
            simple_websockets::Event::Connect(_client_id, _response) => {
                panic!("Connect event received but expected text message event")
            }
            simple_websockets::Event::Disconnect(_client_id) => {
                panic!("Disconnect event received but expected text message event")
            }
            simple_websockets::Event::Message(client_id, message) => {
                assert_eq!(expected_client_id, client_id);
                match message {
                    simple_websockets::Message::Text(msg) => {
                        assert_eq!(expected_text_content, msg);
                    }
                    simple_websockets::Message::Binary(_) => {
                        panic!("Binary message received but expected text message")
                    }
                }
            }
        }
    }

    fn assert_binary_message_event(
        e: simple_websockets::Event,
        expected_client_id: u64,
        expected_binary_content: Vec<u8>,
    ) {
        match e {
            simple_websockets::Event::Connect(_client_id, _response) => {
                panic!("Connect event received but expected binary message event")
            }
            simple_websockets::Event::Disconnect(_client_id) => {
                panic!("Disconnect event received but expected binary message event")
            }
            simple_websockets::Event::Message(client_id, message) => {
                assert_eq!(expected_client_id, client_id);
                match message {
                    simple_websockets::Message::Text(_) => {
                        panic!("Text message received but expected text message")
                    }
                    simple_websockets::Message::Binary(bytes) => {
                        assert_eq!(expected_binary_content.len(), bytes.len());
                        let mut i = 0;
                        while i < expected_binary_content.len() {
                            assert_eq!(expected_binary_content[i], bytes[i]);
                            i += 1;
                        }
                    }
                }
            }
        }
    }
}
