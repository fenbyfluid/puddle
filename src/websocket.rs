use crate::CoreEvent;
use anyhow::Result;
use log::{debug, error, info, trace, warn};
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token, Waker};
use puddle::ControllerId;
use puddle::messages::{ClientMessage, CoreMessage};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc;
use tungstenite::handshake::MidHandshake;
use tungstenite::handshake::server::{NoCallback, ServerHandshake};
use tungstenite::protocol::WebSocket;
use tungstenite::{HandshakeError, Message};

const SERVER: Token = Token(0);
const WAKER: Token = Token(1);

pub struct Server {
    #[cfg(test)]
    local_addr: SocketAddr,
    waker: Arc<Waker>,
    outbound_sender: mpsc::Sender<(Option<Token>, CoreMessage)>,
}

impl Server {
    pub fn new(port: u16, inbound_sender: mpsc::Sender<CoreEvent>) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port).into())?;

        let local_addr = listener.local_addr()?;
        info!("WebSocket server listening on {}", local_addr);

        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), WAKER)?);
        let (outbound_sender, outbound_receiver) = mpsc::channel();

        std::thread::spawn(move || {
            if let Err(e) = run_event_loop(listener, poll, outbound_receiver, inbound_sender) {
                error!("Event loop error: {}", e);
            }
        });

        Ok(Self {
            #[cfg(test)]
            local_addr,
            waker,
            outbound_sender,
        })
    }

    #[cfg(test)]
    pub fn port(&self) -> u16 {
        self.local_addr.port()
    }

    pub fn send(&self, destination: Option<Token>, message: CoreMessage) -> Result<()> {
        self.outbound_sender.send((destination, message))?;
        self.waker.wake()?;
        Ok(())
    }
}

fn run_event_loop(
    mut listener: TcpListener,
    mut poll: Poll,
    outbound_receiver: mpsc::Receiver<(Option<Token>, CoreMessage)>,
    inbound_sender: mpsc::Sender<CoreEvent>,
) -> Result<()> {
    let mut events = Events::with_capacity(128);
    let mut clients: HashMap<Token, Client> = HashMap::new();
    let mut next_token = 2;

    poll.registry().register(&mut listener, SERVER, Interest::READABLE)?;

    loop {
        poll.poll(&mut events, None)?;

        for event in events.iter() {
            match event.token() {
                SERVER => accept_connections(&mut listener, &poll, &mut clients, &mut next_token, &inbound_sender)?,
                WAKER => handle_outbound(&outbound_receiver, &mut clients, &inbound_sender)?,
                token => handle_client_event(token, &mut clients, &mut poll, &inbound_sender)?,
            }
        }
    }
}

fn accept_connections(
    listener: &mut TcpListener,
    poll: &Poll,
    clients: &mut HashMap<Token, Client>,
    next_token: &mut usize,
    inbound_sender: &mpsc::Sender<CoreEvent>,
) -> Result<()> {
    loop {
        match listener.accept() {
            Ok((mut stream, addr)) => {
                info!("Accepted a new WebSocket connection from {:?}", addr);
                let token = Token(*next_token);
                *next_token += 1;

                poll.registry().register(&mut stream, token, Interest::READABLE | Interest::WRITABLE)?;

                match tungstenite::accept(stream) {
                    Ok(ws) => {
                        info!("WebSocket handshake successful for {:?}", addr);
                        clients.insert(token, Client::Connected(ws, addr));
                        inbound_sender.send(CoreEvent::Connected { controller_id: ControllerId::WebSocket(token) })?;
                    }
                    Err(HandshakeError::Interrupted(mid)) => {
                        clients.insert(token, Client::Handshaking(mid, addr));
                    }
                    Err(e) => {
                        error!("Handshake failed for {:?}: {}", addr, e);
                    }
                }
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn handle_outbound(
    receiver: &mpsc::Receiver<(Option<Token>, CoreMessage)>,
    clients: &mut HashMap<Token, Client>,
    inbound_sender: &mpsc::Sender<CoreEvent>,
) -> Result<()> {
    while let Ok((token, message)) = receiver.try_recv() {
        let mut to_remove = Vec::new();

        // TODO: We may want to use binary messages for our HID debug mode and Feedback messages.
        let body = serde_json::to_string(&message)?;
        let to_send = Message::text(body);

        if let Some(token) = token {
            if let Some(client) = clients.get_mut(&token) {
                debug!("Sending message to {:?}: {:?}", token, message);

                if let Client::Connected(ws, addr) = client {
                    if let Err(e) = ws.send(to_send) {
                        error!("Failed to send message to {:?}: {}", addr, e);
                        to_remove.push(token);
                    }
                }
            } else {
                error!("Failed to send message to unknown client {:?}", token);
            }
        } else {
            if !clients.is_empty() {
                match &message {
                    CoreMessage::State { .. } => {
                        trace!("Broadcasting message to {} clients: {:?}", clients.len(), message)
                    }
                    _ => debug!("Broadcasting message to {} clients: {:?}", clients.len(), message),
                }
            }

            for (token, client) in clients.iter_mut() {
                if let Client::Connected(ws, addr) = client {
                    if let Err(e) = ws.send(to_send.clone()) {
                        error!("Failed to send broadcast to {:?}: {}", addr, e);
                        to_remove.push(*token);
                    }
                }
            }
        }

        for token in to_remove {
            clients.remove(&token);
            inbound_sender.send(CoreEvent::Disconnected { controller_id: ControllerId::WebSocket(token) })?;
        }
    }

    Ok(())
}

fn handle_client_event(
    token: Token,
    clients: &mut HashMap<Token, Client>,
    poll: &mut Poll,
    inbound_sender: &mpsc::Sender<CoreEvent>,
) -> Result<()> {
    if let Some(client) = clients.remove(&token) {
        match client.process(poll, token, inbound_sender) {
            Ok(Some(new_client)) => {
                clients.insert(token, new_client);
            }
            Ok(None) => {
                // Client closed
            }
            Err(e) => {
                error!("Error processing client {:?}: {}", token, e);
            }
        }
    }
    Ok(())
}

enum Client {
    Handshaking(MidHandshake<ServerHandshake<TcpStream, NoCallback>>, SocketAddr),
    Connected(WebSocket<TcpStream>, SocketAddr),
}

impl Client {
    fn process(self, poll: &mut Poll, token: Token, inbound_sender: &mpsc::Sender<CoreEvent>) -> Result<Option<Self>> {
        match self {
            Client::Handshaking(mid, addr) => match mid.handshake() {
                Ok(ws) => {
                    info!("WebSocket handshake successful for {:?}", addr);
                    inbound_sender.send(CoreEvent::Connected { controller_id: ControllerId::WebSocket(token) })?;
                    Self::Connected(ws, addr).process(poll, token, inbound_sender)
                }
                Err(HandshakeError::Interrupted(mid)) => Ok(Some(Client::Handshaking(mid, addr))),
                Err(e) => {
                    error!("Handshake failed for {:?}: {}", addr, e);
                    Ok(None)
                }
            },
            Client::Connected(mut ws, addr) => loop {
                match ws.read() {
                    Ok(Message::Text(msg)) => {
                        trace!("Received a WebSocket text message from {:?}: {}", addr, msg);

                        match serde_json::from_str::<ClientMessage>(&msg) {
                            Ok(message) => {
                                debug!("Parsed message: {:?}", message);
                                inbound_sender.send(CoreEvent::Message {
                                    controller_id: ControllerId::WebSocket(token),
                                    message,
                                })?;
                            }
                            Err(e) => {
                                // TODO: Consider sending a message back to the client, an ack with
                                //       success=false would be a good fit, but we'd need to try and
                                //       get a seq from the inbound message, and skip the outbound
                                //       seq if there was none.
                                error!("Failed to parse message: {}", e);
                            }
                        }
                    }
                    Ok(Message::Binary(msg)) => {
                        // TODO: We may be interested in using binary messages for our HID debug mode.
                        warn!("Received a WebSocket binary message from {:?}: {:?}", addr, msg);
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {
                        return Ok(Some(Client::Connected(ws, addr)));
                    }
                    Err(tungstenite::Error::ConnectionClosed) => {
                        info!("Closed a WebSocket connection from {:?}", addr);
                        inbound_sender
                            .send(CoreEvent::Disconnected { controller_id: ControllerId::WebSocket(token) })?;
                        return Ok(None);
                    }
                    Err(e) => {
                        error!("WebSocket error for {:?}: {}", addr, e);
                        inbound_sender
                            .send(CoreEvent::Disconnected { controller_id: ControllerId::WebSocket(token) })?;
                        return Ok(None);
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_websocket_tcp_close() {
        let _ = env_logger::builder().is_test(true).try_init();

        let (inbound_sender, inbound_receiver) = mpsc::channel();

        let server = Server::new(0, inbound_sender).expect("Failed to start server");

        {
            let stream =
                std::net::TcpStream::connect(format!("localhost:{}", server.port())).expect("Failed to connect");
            let (socket, _) =
                tungstenite::client(format!("ws://localhost:{}", server.port()), stream).expect("Handshake failed");

            // Wait for Connected event
            let event = inbound_receiver.recv().expect("Failed to receive event");
            let token = match event {
                CoreEvent::Connected { controller_id: ControllerId::WebSocket(token) } => token,
                _ => panic!("Unexpected event: {:?}", event),
            };

            // Drop the socket WITHOUT closing it gracefully
            drop(socket);

            // Wait for Disconnected event
            let event = inbound_receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("Failed to receive disconnect event");
            match event {
                CoreEvent::Disconnected { controller_id: ControllerId::WebSocket(t) } => assert_eq!(t, token),
                _ => panic!("Unexpected event: {:?}", event),
            }
        }
    }

    #[test]
    fn test_websocket_connection_close() {
        let _ = env_logger::builder().is_test(true).try_init();

        let (dummy_sender, _dummy_receiver) = mpsc::channel();

        let server = Server::new(0, dummy_sender).expect("Failed to start server");

        let (mut socket, _) =
            tungstenite::connect(format!("ws://localhost:{}", server.port())).expect("Failed to connect");

        socket.close(None).unwrap();
        loop {
            match socket.read() {
                Ok(_) => continue,
                Err(tungstenite::Error::ConnectionClosed) => break,
                Err(e) => panic!("Unexpected error: {:?}", e),
            }
        }
    }
}
