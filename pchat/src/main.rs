// mod cli; // 命令行支持

use async_std::io;
use futures::{future::Either, prelude::*, select};
use libp2p::{
    core::{muxing::StreamMuxerBox, transport::OrTransport, upgrade},
    gossipsub::{self}, mdns, noise,
    swarm::{SwarmBuilder, SwarmEvent},
    tcp, yamux, Transport,
};
use libp2p_quic as quic;
use std::error::Error;
use pchat::behaviour::*;
use pchat_utils::message_id_generator::MessageIdGenerator;
use pchat_account::Account;


// 创建了一个结合了 Gossipsub 和 Mdns 的自定义网络行为。 

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("start p2p-chat");
    // 创建账号
    let user = Account::new();
    
    println!("Local peer id: {}", user.peer_id);

    // 通过 Mplex 协议设置加密的启用 DNS 的 TCP 传输。
    let tcp_transport = tcp::async_io::Transport::new(tcp::Config::default().nodelay(true))
        .upgrade(upgrade::Version::V1)
        .authenticate(
            noise::NoiseAuthenticated::xx(&user.id_keys).expect("signing libp2p-noise static keypair"),
        )
        .multiplex(yamux::YamuxConfig::default())
        .timeout(std::time::Duration::from_secs(20))
        .boxed();
    let quic_transport = quic::tokio::Transport::new(quic::Config::new(&user.id_keys));
    let transport = OrTransport::new(quic_transport, tcp_transport)
        .map(|either_output, _| match either_output {
            Either::Left((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
            Either::Right((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
        })
        .boxed();

    // init message id options
    MessageIdGenerator::init();
    // Create a Gossipsub topic
    let topic = gossipsub::IdentTopic::new("test-net");
    // // subscribes to our topic
    // gossipsub.subscribe(&topic)?;

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let behaviour = ChatBehaviour::new(user.clone());
        SwarmBuilder::with_async_std_executor(transport, behaviour, user.peer_id).build()
    };

    // 从 stdin 读取整行
    let mut stdin = io::BufReader::new(io::stdin()).lines().fuse();

    //监听所有接口和操作系统分配的任何端口
    swarm.listen_on(user.address)?;

    println!("Enter messages via STDIN and they will be sent to connected peers using Gossipsub");

    // Kick it off
    loop {
        select! {
            line = stdin.select_next_some() => {
                if let Err(e) = swarm
                    .behaviour_mut().gossipsub
                    .publish(topic.clone(), line.expect("Stdin not to close").as_bytes()) {
                    println!("Publish error: {e:?}");
                }
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discovered a new peer: {peer_id}");
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discover peer has expired: {peer_id}");
                        swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(ChatBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                    propagation_source: peer_id,
                    message_id: id,
                    message,
                })) => println!(
                        "Got message: '{}' with id: {id} from peer: {peer_id}",
                        String::from_utf8_lossy(&message.data),
                    ),
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Local node is listening on {address}");
                },
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("Connected to {}", peer_id);
                },
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    println!("Disconnected from {}", peer_id);
                },
                SwarmEvent::IncomingConnection { .. } => {
                    println!("Incoming connection");
                },
                SwarmEvent::IncomingConnectionError { .. } => {
                    println!("Incoming connection error");
                },
                _ => {}
            }
        }
    }
}
