#![allow(dead_code)]

use std::option::Option;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::thread;
use env_logger;

use rocksdb::DB;
use tempdir::TempDir;
use uuid::Uuid;
use protobuf;

use tikv::raftserver::store::*;
use tikv::raftserver::{Result, other};
use tikv::proto::metapb;
use tikv::proto::raft_serverpb;
use tikv::proto::raft_cmdpb::{Request, RaftCommandRequest, RaftCommandResponse};
use tikv::proto::raft_cmdpb::CommandType;
use tikv::raft::INVALID_ID;

pub struct StoreTransport {
    peers: HashMap<u64, metapb::Peer>,

    senders: HashMap<u64, Sender>,
}

impl StoreTransport {
    pub fn new() -> Arc<RwLock<StoreTransport>> {
        Arc::new(RwLock::new(StoreTransport {
            peers: HashMap::new(),
            senders: HashMap::new(),
        }))
    }

    pub fn add_sender(&mut self, store_id: u64, sender: Sender) {
        self.senders.insert(store_id, sender);
    }

    pub fn remove_sender(&mut self, store_id: u64) {
        self.senders.remove(&store_id);
    }
}

impl Transport for StoreTransport {
    fn cache_peer(&mut self, peer_id: u64, peer: metapb::Peer) {
        self.peers.insert(peer_id, peer);
    }

    fn get_peer(&self, peer_id: u64) -> Option<metapb::Peer> {
        self.peers.get(&peer_id).cloned()
    }

    fn send(&self, msg: raft_serverpb::RaftMessage) -> Result<()> {
        let to_store = msg.get_to_peer().get_store_id();
        match self.senders.get(&to_store) {
            None => Err(other(format!("missing sender for store {}", to_store))),
            Some(sender) => sender.send_raft_msg(msg),
        }
    }
}

pub fn new_engine(path: &TempDir) -> Arc<DB> {
    let db = DB::open_default(path.path().to_str().unwrap()).unwrap();
    Arc::new(db)
}

pub fn new_store(engine: Arc<DB>, trans: Arc<RwLock<StoreTransport>>) -> Store<StoreTransport> {
    let store = Store::new(Config::default(), engine, trans.clone()).unwrap();

    trans.write().unwrap().add_sender(store.get_store_id(), store.get_sender());

    store
}

// Create a base request.
pub fn new_base_request(region_id: u64, peer: metapb::Peer) -> RaftCommandRequest {
    let mut req = RaftCommandRequest::new();
    req.mut_header().set_region_id(region_id);
    req.mut_header().set_peer(peer);
    req.mut_header().set_uuid(Uuid::new_v4().as_bytes().to_vec());
    req
}

pub fn new_request(region_id: u64,
                   peer: metapb::Peer,
                   requests: Vec<Request>)
                   -> RaftCommandRequest {
    let mut req = new_base_request(region_id, peer);
    req.set_requests(protobuf::RepeatedField::from_vec(requests));
    req
}

pub fn new_put_cmd(key: &[u8], value: &[u8]) -> Request {
    let mut cmd = Request::new();
    cmd.set_cmd_type(CommandType::Put);
    cmd.mut_put().set_key(key.to_vec());
    cmd.mut_put().set_value(value.to_vec());
    cmd
}

pub fn new_get_cmd(key: &[u8]) -> Request {
    let mut cmd = Request::new();
    cmd.set_cmd_type(CommandType::Get);
    cmd.mut_get().set_key(key.to_vec());
    cmd
}

pub fn new_delete_cmd(key: &[u8]) -> Request {
    let mut cmd = Request::new();
    cmd.set_cmd_type(CommandType::Delete);
    cmd.mut_delete().set_key(key.to_vec());
    cmd
}

pub fn new_seek_cmd(key: &[u8]) -> Request {
    let mut cmd = Request::new();
    cmd.set_cmd_type(CommandType::Seek);
    cmd.mut_seek().set_key(key.to_vec());
    cmd
}

pub fn new_peer(node_id: u64, store_id: u64, peer_id: u64) -> metapb::Peer {
    let mut peer = metapb::Peer::new();
    peer.set_node_id(node_id);
    peer.set_store_id(store_id);
    peer.set_peer_id(peer_id);
    peer
}

pub fn sleep_ms(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

// A help function to simplify using env_logger.
pub fn init_env_log() {
    env_logger::init().expect("");
}

pub fn is_error_response(resp: &RaftCommandResponse) -> bool {
    resp.get_header().has_error()
}

// If the resp is "not leader error", get the real leader.
// Sometimes, we may still can't get leader even in "not leader error",
// returns a INVALID_PEER for this.
pub fn check_not_leader_error(resp: &RaftCommandResponse) -> Option<metapb::Peer> {
    if !is_error_response(resp) {
        return None;
    }

    let err = resp.get_header().get_error().get_detail();
    if !err.has_not_leader() {
        return None;
    }

    let err = err.get_not_leader();
    if err.has_leader() {
        return Some(new_peer(INVALID_ID, INVALID_ID, INVALID_ID));
    }

    Some(err.get_leader().clone())
}

pub fn is_invalid_peer(peer: &metapb::Peer) -> bool {
    peer.get_peer_id() == INVALID_ID
}
