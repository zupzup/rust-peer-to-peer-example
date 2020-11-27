use futures::prelude::*;
use libp2p::{
    core::upgrade,
    floodsub::{Floodsub, FloodsubEvent, Topic},
    identity,
    mdns::{MdnsEvent, TokioMdns},
    mplex,
    noise::{Keypair, NoiseConfig, X25519Spec},
    swarm::{NetworkBehaviourEventProcess, Swarm, SwarmBuilder},
    tcp::TokioTcpConfig,
    Multiaddr, NetworkBehaviour, PeerId, Transport,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;

const TOPIC: &str = "recipes";

#[derive(NetworkBehaviour)]
struct RecipeBehaviour {
    floodsub: Floodsub,
    mdns: TokioMdns,
}

#[derive(Debug, Serialize, Deserialize)]
struct Recipe {
    id: usize,
    name: String,
    ingredients: String,
    instructions: String,
    public: bool,
}

#[derive(Debug, Serialize, Deserialize)]
enum ListMode {
    ALL,
    One(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct ListRequest {
    mode: ListMode,
}

#[derive(Debug, Serialize, Deserialize)]
struct ListResponse {
    mode: ListMode,
    data: Vec<Recipe>,
}

impl NetworkBehaviourEventProcess<FloodsubEvent> for RecipeBehaviour {
    fn inject_event(&mut self, event: FloodsubEvent) {
        match event {
            FloodsubEvent::Message(msg) => {
                // TODO: react to list requests
                println!(
                    "Received {:?} from {:?}",
                    String::from_utf8_lossy(&msg.data),
                    msg.source
                );
            }
            FloodsubEvent::Subscribed { peer_id, topic } => {
                println!("Subscribed event on {:?} from {:?}", topic, peer_id)
            }
            FloodsubEvent::Unsubscribed { peer_id, topic } => {
                println!("Unsubscribed event on {:?} from {:?}", topic, peer_id)
            }
        }
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for RecipeBehaviour {
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            MdnsEvent::Discovered(discovered_list) => {
                for (peer, _addr) in discovered_list {
                    // println!("Discovered {:?} with addr {:?}", peer, addr);
                    self.floodsub.add_node_to_partial_view(peer);
                }
            }
            MdnsEvent::Expired(expired_list) => {
                for (peer, _addr) in expired_list {
                    // println!("Expired {:?} with addr {:?}", peer, addr);
                    if !self.mdns.has_node(&peer) {
                        self.floodsub.remove_node_from_partial_view(&peer);
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let keys = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(keys.public());
    println!("Peer Id: {}", peer_id);

    let auth_keys = Keypair::<X25519Spec>::new()
        .into_authentic(&keys)
        .expect("can create auth keys");

    let topic = Topic::new(TOPIC);

    let transp = TokioTcpConfig::new()
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(auth_keys).into_authenticated()) // XX Handshake pattern, IX exists as well and IK - only XX currently provides interop with other libp2p impls
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    let mut behaviour = RecipeBehaviour {
        floodsub: Floodsub::new(peer_id.clone()),
        mdns: TokioMdns::new().expect("can create mdns"),
    };

    behaviour.floodsub.subscribe(topic.clone());

    let mut swarm = SwarmBuilder::new(transp, behaviour, peer_id)
        .executor(Box::new(|fut| {
            tokio::spawn(fut);
        }))
        .build();

    if let Some(to_dial) = std::env::args().nth(1) {
        let addr: Multiaddr = to_dial.parse().expect("provide a valid addr");
        Swarm::dial_addr(&mut swarm, addr).expect("can connect to given node");
        println!("Dialed {:?}", to_dial)
    }

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    Swarm::listen_on(
        &mut swarm,
        "/ip4/0.0.0.0/tcp/0"
            .parse()
            .expect("can get a local socket"),
    )
    .expect("swarm can be started");

    // TODO: rewrite this
    loop {
        let to_publish = {
            tokio::select! { // TODO: select waits for multiple events at the same time
                line = stdin.try_next() => Some(line.expect("line").expect("Stdin closed")),
                event = swarm.next() => {
                    println!("New Event: {:?}", event);
                    None
                }
            }
        };
        if let Some(line) = to_publish {
            // There is input - parse it
            // Options:
            // * ls p -> list peers with indices next to them
            // * ls r {peer_index} -> list recipes with ids next to them, if peer_index is not set, display own, send list to given peer, wait 100 ms
            // * ls r all -> list all reachable recipes - send list to all peers, wait 100 ms
            // * create r {name} {ingredients} {instructions}
            // * publish r {recipe_id}, send to recipe topic
            swarm.floodsub.publish(topic.clone(), line.as_bytes());
        }
    }
}
