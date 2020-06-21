use bigint::U256;
use core::convert::TryFrom;
use rand::Rng;
use std::collections::HashMap;
use std::io::prelude::*;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::SystemTime;

const CHUNK_SIZE: u32 = 50000;

type Nonce = u32;
type ResourceTarget = [u8; 32];
type Args = HashMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ResourceKind {
    COAL,
    IRON,
    GOLD,
    DIAMOND,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Resource {
    target: ResourceTarget,
    kind: ResourceKind,
}

impl Resource {
    fn new(kind: ResourceKind, v: &str) -> Self {
        let mut s_buf = hex::decode(v).unwrap();
        s_buf.reverse();

        let n = U256::try_from(s_buf.as_slice()).unwrap();

        let mut n_buf = [0; 32];
        n.to_big_endian(&mut n_buf);

        Self {
            target: n_buf,
            kind: kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResourceMatcher {
    resources: Vec<Resource>,
}

impl ResourceMatcher {
    fn new() -> Self {
        let mut resources = vec![];

        resources.push(Resource::new(ResourceKind::DIAMOND, "000000000fff"));
        resources.push(Resource::new(ResourceKind::GOLD, "00000002ffff"));
        resources.push(Resource::new(ResourceKind::IRON, "0000000fffff"));
        resources.push(Resource::new(ResourceKind::COAL, "0000003fffff"));

        Self {
            resources: resources,
        }
    }

    fn match_hash(&self, hash: &[u8]) -> Option<Resource> {
        for resource in self.resources.iter() {
            let mut brk = false;
            for i in (0..hash.len()).rev() {
                if hash[i] < resource.target[i] {
                    return Some(resource.clone());
                }
                if hash[i] > resource.target[i] {
                    brk = true;
                    break;
                }
            }
            if !brk {
                return Some(resource.clone());
            }
        }
        None
    }
}

fn get_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

#[derive(Debug, Clone)]
struct Block {
    created_at: u128,
    seed: [u8; 32],
}

impl Block {
    fn new() -> Self {
        Self {
            created_at: get_time_ms(),
            seed: rand::thread_rng().gen::<[u8; 32]>(),
        }
    }

    fn seed_hex(&self) -> String {
        hex::encode(self.seed)
    }

    fn hash(&self, nonce: Nonce) -> Vec<u8> {
        let mut hasher = blake3::Hasher::new();

        hasher.update(&self.created_at.to_be_bytes());
        hasher.update(&self.seed);
        hasher.update(&nonce.to_be_bytes());

        let res = hasher.finalize();

        res.as_bytes().to_vec()
    }
}

struct BlockManager {
    blocks: [(Block, Nonce); 2],
    mined: u128,
}

impl BlockManager {
    fn new() -> Self {
        Self {
            blocks: [(Block::new(), 0), (Block::new(), 0)],
            mined: 0,
        }
    }

    fn get_block(&mut self) -> (Block, u32, u32) {
        let b = &self.blocks[0].clone();
        let start = b.1;

        if start == u32::MAX {
            self.blocks = [self.blocks[1].clone(), (Block::new(), 0)];
            self.mined += 1;

            return self.get_block();
        }

        let mut left = u32::MAX - start;
        if left > CHUNK_SIZE {
            left = CHUNK_SIZE;
        }
        let end = start + left;
        self.blocks[0].1 += left;

        (b.0.clone(), start, end)
    }
}

fn get_args() -> Args {
    let mut args = Args::new();
    let args_tmp: Vec<String> = std::env::args().collect();

    for a in args_tmp {
        if let Some(argument) = a.strip_prefix("-") {
            let argument_value: Vec<String> =
                argument.splitn(2, "=").map(|s| s.to_owned()).collect();
            if argument_value.len() != 2 {
                panic!("invalid argument: {}", argument);
            }

            args.insert(argument_value[0].clone(), argument_value[1].clone());
        }
    }

    args
}

fn main() {
    let mut save_file = std::fs::File::create("mined.txt").unwrap();

    let args = get_args();

    let (tx, rx) = mpsc::channel();

    let block_manager = Arc::new(Mutex::new(BlockManager::new()));
    let resource_manager = ResourceMatcher::new();
    let threads = if let Some(threads) = args.get("threads") {
        threads.parse::<usize>().unwrap()
    } else {
        num_cpus::get_physical()
    };
    let mut handles = vec![];

    println!("start: {:#?}", get_time_ms());
    println!("threads: {}", threads);
    println!("mining...");

    for _ in 0..threads {
        let bm = block_manager.clone();
        let rm = resource_manager.clone();
        let tx = tx.clone();
        handles.push(thread::spawn(move || loop {
            let (block, start, end) = {
                let mut bm = bm.lock().unwrap();
                bm.get_block()
            };

            let mut n = start;
            loop {
                let hash = block.hash(n);

                if let Some(resource) = rm.match_hash(hash.as_slice()) {
                    let _ = tx.send((resource, hex::encode(hash)));
                }

                if n == end {
                    break;
                }

                n += 1;
            }
        }));
    }

    thread::spawn(move || {
        let mut mined: HashMap<ResourceKind, u128> = HashMap::new();
        let bm = block_manager.clone();

        loop {
            match rx.recv() {
                Ok((resource, hash)) => {
                    if let Some(v) = mined.get_mut(&resource.kind) {
                        *v += 1;
                    } else {
                        mined.insert(resource.kind.clone(), 1);
                    }

                    save_file
                        .write_fmt(format_args!(
                            "{} {:?} {}\n",
                            get_time_ms(),
                            resource.kind,
                            hash
                        ))
                        .unwrap();

                    let bm = bm.lock().unwrap();

                    print!("\r{:?} BLOCKS: {}", mined, bm.mined);
                    std::io::stdout()
                        .flush()
                        .ok()
                        .expect("Couldn't flush stdout!");
                }
                Err(_) => break,
            }
        }
    });

    for handle in handles {
        let _ = handle.join();
    }
}
