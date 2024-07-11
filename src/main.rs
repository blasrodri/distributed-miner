use distributed_drillx::{get_hash, MasterNode, NodeHashComputer};
use structopt::StructOpt;

fn main() {
    env_logger::init();
    let opt = NodeType::from_args();

    match opt {
        NodeType::Master { host } => {
            MasterNode::start_websocket_server(host);
        }
        NodeType::Node { master } => {
            let mut socket = NodeHashComputer::connect(master).unwrap();
            // move this to its own function
            loop {
                let challenge = NodeHashComputer::receive_challenge(&mut socket);

                let solution = get_hash(challenge);
                let solution = [solution.d.as_slice(), solution.h.as_slice()].concat();
                NodeHashComputer::send_solution(&mut socket, solution);
            }
        }
    }
}

#[derive(Debug, StructOpt)]
enum NodeType {
    Master {
        #[structopt(short = "h", long = "host", default_value = "127.0.0.1")]
        host: String,
    },
    Node {
        #[structopt(short = "m", long = "master", default_value = "127.0.0.1")]
        master: String,
    },
}
