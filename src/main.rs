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
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tokio::{fs, io::AsyncBufReadExt};

static KEYS: Lazy<identity::Keypair> = Lazy::new(|| identity::Keypair::generate_ed25519());

static PEER_ID: Lazy<PeerId> = Lazy::new(|| PeerId::from(KEYS.public()));

const TOPIC: &str = "recipes";
const STORAGE_FILE_PATH: &str = "./recipes.json";

#[derive(NetworkBehaviour)]
struct RecipeBehaviour {
    floodsub: Floodsub,
    mdns: TokioMdns,
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;
type Recipes = Vec<Recipe>;

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
    receiver: String,
}

impl NetworkBehaviourEventProcess<FloodsubEvent> for RecipeBehaviour {
    fn inject_event(&mut self, event: FloodsubEvent) {
        match event {
            FloodsubEvent::Message(msg) => {
                if let Ok(req) = serde_json::from_slice::<ListRequest>(&msg.data) {
                    match req.mode {
                        ListMode::ALL => {
                            println!("Received ALL req: {:?} from {:?}", req, msg.source);
                            match read_local_recipes().await { // TODO: how to do async stuff here, tokio::spawn?
                                Ok(recipes) => {
                                    // TODO: send back
                                }
                                Err(e) => eprintln!("error fetching local recipes to answer ALL request");
                            }
                        }
                        ListMode::One(ref peer_id) => {
                            if peer_id == &PEER_ID.to_string() {
                                println!("Received req: {:?} from {:?}", req, msg.source);
                            }
                        }
                    }
                } else if let Ok(resp) = serde_json::from_slice::<ListResponse>(&msg.data) {
                    if resp.receiver == PEER_ID.to_string() {
                        println!("Received resp: {:?} from {:?}", resp, msg.source);
                    }
                }
            }
            _ => (),
        }
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for RecipeBehaviour {
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            MdnsEvent::Discovered(discovered_list) => {
                for (peer, _addr) in discovered_list {
                    self.floodsub.add_node_to_partial_view(peer);
                }
            }
            MdnsEvent::Expired(expired_list) => {
                for (peer, _addr) in expired_list {
                    if !self.mdns.has_node(&peer) {
                        self.floodsub.remove_node_from_partial_view(&peer);
                    }
                }
            }
        }
    }
}

async fn create_new_recipe(name: &str, ingredients: &str, instructions: &str) -> Result<()> {
    let mut local_recipes = read_local_recipes().await?;
    let new_id = match local_recipes.iter().max_by_key(|r| r.id) {
        Some(v) => v.id + 1,
        None => 0,
    };
    local_recipes.push(Recipe {
        id: new_id,
        name: name.to_owned(),
        ingredients: ingredients.to_owned(),
        instructions: instructions.to_owned(),
        public: false,
    });
    write_local_recipes(&local_recipes).await?;

    println!("Created recipe:");
    println!("Name: {}", name);
    println!("Ingredients: {}", ingredients);
    println!("Instructions:: {}", instructions);

    Ok(())
}

async fn publish_recipe(id: usize) -> Result<()> {
    let mut local_recipes = read_local_recipes().await?;
    local_recipes
        .iter_mut()
        .filter(|r| r.id == id)
        .for_each(|r| r.public = true);
    write_local_recipes(&local_recipes).await?;
    Ok(())
}

async fn read_local_recipes() -> Result<Recipes> {
    let content = fs::read(STORAGE_FILE_PATH).await?;
    let result = serde_json::from_slice(&content)?;
    Ok(result)
}

async fn write_local_recipes(recipes: &Recipes) -> Result<()> {
    let json = serde_json::to_string(&recipes)?;
    fs::write(STORAGE_FILE_PATH, &json).await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    println!("Peer Id: {}", PEER_ID.clone());

    let auth_keys = Keypair::<X25519Spec>::new()
        .into_authentic(&KEYS)
        .expect("can create auth keys");

    let topic = Topic::new(TOPIC);

    let transp = TokioTcpConfig::new()
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(auth_keys).into_authenticated()) // XX Handshake pattern, IX exists as well and IK - only XX currently provides interop with other libp2p impls
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    let mut behaviour = RecipeBehaviour {
        floodsub: Floodsub::new(PEER_ID.clone()),
        mdns: TokioMdns::new().expect("can create mdns"),
    };

    behaviour.floodsub.subscribe(topic.clone());

    let mut swarm = SwarmBuilder::new(transp, behaviour, PEER_ID.clone())
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

    loop {
        let evt = {
            tokio::select! {
                line = stdin.next_line() => Some(line.expect("can get line").expect("can read line from stdin")),
                event = swarm.next() => {
                    println!("Swarm Event: {:?}", event);
                    None
                }
            }
        };
        if let Some(line) = evt {
            match line.as_str() {
                "ls p" => {
                    println!("Discovered Peers:");
                    let nodes = swarm.mdns.discovered_nodes();
                    let mut unique_peers = HashSet::new();
                    for peer in nodes {
                        unique_peers.insert(peer);
                    }
                    unique_peers.iter().for_each(|p| println!("{}", p));
                }
                cmd if cmd.starts_with("ls r ") => {
                    let rest = cmd.strip_prefix("ls r ");
                    match rest {
                        Some("all") => {
                            let req = ListRequest {
                                mode: ListMode::ALL,
                            };
                            let json = serde_json::to_string(&req).expect("can jsonify request");
                            swarm.floodsub.publish(topic.clone(), json.as_bytes());

                            println!("waiting for responses...");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            println!("end of waiting");
                        }
                        Some(recipes_peer_id) => {
                            let req = ListRequest {
                                mode: ListMode::One(recipes_peer_id.to_owned()),
                            };
                            let json = serde_json::to_string(&req).expect("can jsonify request");
                            swarm.floodsub.publish(topic.clone(), json.as_bytes());

                            println!("waiting for response...");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            println!("end of waiting");
                        }
                        None => {
                            match read_local_recipes().await {
                                Ok(v) => {
                                    println!("Local Recipes ({})", v.len());
                                    v.iter().for_each(|r| println!("{:?}", r));
                                }
                                Err(e) => eprintln!("error fetching local recipes: {}", e),
                            };
                        }
                    };
                }
                cmd if cmd.starts_with("create r") => {
                    if let Some(rest) = cmd.strip_prefix("create r") {
                        let elements: Vec<&str> = rest.split("|").collect();
                        if elements.len() < 3 {
                            println!("too few arguments - Format: name|ingredients|instructions");
                        } else {
                            let name = elements.get(0).expect("name is there");
                            let ingredients = elements.get(1).expect("ingredients is there");
                            let instructions = elements.get(2).expect("instructions is there");
                            if let Err(e) = create_new_recipe(name, ingredients, instructions).await
                            {
                                eprintln!("error creating recipe: {}", e);
                            };
                        }
                    }
                }
                cmd if cmd.starts_with("publish r") => {
                    if let Some(rest) = cmd.strip_prefix("publish r") {
                        match rest.trim().parse::<usize>() {
                            Ok(id) => {
                                if let Err(e) = publish_recipe(id).await {
                                    println!("error publishing recipe with id {}, {}", id, e)
                                } else {
                                    println!("Published Recipe with id: {}", id);
                                }
                            }
                            Err(e) => eprintln!("invalid id: {}, {}", rest.trim(), e),
                        };
                    }
                }
                _ => eprintln!("unexpected command"),
            };
        }
    }
}
