use std::fmt;

use darkfi_sdk::crypto::{constants::MERKLE_DEPTH, MerkleNode};
use darkfi_serial::{serialize, SerialDecodable, SerialEncodable};
use incrementalmerkletree::{bridgetree::BridgeTree, Tree};
use log::debug;
use pasta_curves::pallas;

use super::{Metadata, BLOCK_MAGIC_BYTES, BLOCK_VERSION};
use crate::{net, tx::Transaction, util::time::Timestamp};

/// This struct represents a tuple of the form (version, previous, epoch, slot, timestamp, merkle_root).
#[derive(Debug, Clone, PartialEq, Eq, SerialEncodable, SerialDecodable)]
pub struct Header {
    /// Block version
    pub version: u8,
    /// Previous block hash
    pub previous: blake3::Hash,
    /// Epoch
    pub epoch: u64,
    /// Slot UID
    pub slot: u64,
    /// Block creation timestamp
    pub timestamp: Timestamp,
    /// Root of the transaction hashes merkle tree
    pub root: MerkleNode,
}

impl Header {
    pub fn new(
        previous: blake3::Hash,
        epoch: u64,
        slot: u64,
        timestamp: Timestamp,
        root: MerkleNode,
    ) -> Self {
        let version = *BLOCK_VERSION;
        Self { version, previous, epoch, slot, timestamp, root }
    }

    /// Generate the genesis block.
    pub fn genesis_header(genesis_ts: Timestamp, genesis_data: blake3::Hash) -> Self {
        let tree = BridgeTree::<MerkleNode, MERKLE_DEPTH>::new(100);
        let root = tree.root(0).unwrap();

        Self::new(genesis_data, 0, 0, genesis_ts, root)
    }

    /// Calculate the header hash
    pub fn headerhash(&self) -> blake3::Hash {
        blake3::hash(&serialize(self))
    }
}

impl Default for Header {
    fn default() -> Self {
        Header::new(
            blake3::hash(b""),
            0,
            0,
            Timestamp::current_time(),
            MerkleNode::from(pallas::Base::zero()),
        )
    }
}

/// This struct represents a tuple of the form (`magic`, `header`, `counter`, `txs`, `metadata`).
/// The header and transactions are stored as hashes, serving as pointers to
/// the actual data in the sled database.
#[derive(Debug, Clone, SerialEncodable, SerialDecodable)]
pub struct Block {
    /// Block magic bytes
    pub magic: [u8; 4],
    /// Block header
    pub header: blake3::Hash,
    /// Trasaction hashes
    pub txs: Vec<blake3::Hash>,
    /// Metadata
    pub metadata: Metadata,
}

impl net::Message for Block {
    fn name() -> &'static str {
        "block"
    }
}

impl Block {
    pub fn new(
        previous: blake3::Hash,
        epoch: u64,
        slot: u64,
        txs: Vec<blake3::Hash>,
        root: MerkleNode,
        metadata: Metadata,
    ) -> Self {
        let magic = *BLOCK_MAGIC_BYTES;
        let timestamp = Timestamp::current_time();
        let header = Header::new(previous, epoch, slot, timestamp, root);
        let header = header.headerhash();
        Self { magic, header, txs, metadata }
    }

    /// Generate the genesis block.
    pub fn genesis_block(genesis_ts: Timestamp, genesis_data: blake3::Hash) -> Self {
        let magic = *BLOCK_MAGIC_BYTES;
        let header = Header::genesis_header(genesis_ts, genesis_data);
        let header = header.headerhash();
        let metadata = Metadata::default();
        Self { magic, header, txs: vec![], metadata }
    }

    /// Calculate the block hash
    pub fn blockhash(&self) -> blake3::Hash {
        blake3::hash(&serialize(self))
    }
}

/// Auxiliary structure used for blockchain syncing.
#[derive(Debug, SerialEncodable, SerialDecodable)]
pub struct BlockOrder {
    /// Slot UID
    pub slot: u64,
    /// Block headerhash of that slot
    pub block: blake3::Hash,
}

impl net::Message for BlockOrder {
    fn name() -> &'static str {
        "blockorder"
    }
}

/// Structure representing full block data.
#[derive(Debug, Clone, SerialEncodable, SerialDecodable)]
pub struct BlockInfo {
    /// BlockInfo magic bytes
    pub magic: [u8; 4],
    /// Block header data
    pub header: Header,
    /// Transactions payload
    pub txs: Vec<Transaction>,
    /// Metadata,
    pub metadata: Metadata,
}

impl Default for BlockInfo {
    fn default() -> Self {
        let magic = *BLOCK_MAGIC_BYTES;
        Self { magic, header: Header::default(), txs: vec![], metadata: Metadata::default() }
    }
}

impl net::Message for BlockInfo {
    fn name() -> &'static str {
        "blockinfo"
    }
}

impl BlockInfo {
    pub fn new(header: Header, txs: Vec<Transaction>, metadata: Metadata) -> Self {
        let magic = *BLOCK_MAGIC_BYTES;
        Self { magic, header, txs, metadata }
    }

    /// Calculate the block hash
    pub fn blockhash(&self) -> blake3::Hash {
        let block: Block = self.clone().into();
        block.blockhash()
    }
}

impl From<BlockInfo> for Block {
    fn from(block_info: BlockInfo) -> Self {
        let txs = block_info.txs.iter().map(|x| blake3::hash(&serialize(x))).collect();
        Self {
            magic: block_info.magic,
            header: block_info.header.headerhash(),
            txs,
            metadata: block_info.metadata,
        }
    }
}

/// Auxiliary structure used for blockchain syncing
#[derive(Debug, Clone, SerialEncodable, SerialDecodable)]
pub struct BlockResponse {
    /// Response blocks.
    pub blocks: Vec<BlockInfo>,
}

impl net::Message for BlockResponse {
    fn name() -> &'static str {
        "blockresponse"
    }
}

/// This struct represents a block proposal, used for consensus.
#[derive(Debug, Clone, SerialEncodable, SerialDecodable)]
pub struct BlockProposal {
    /// Block data
    pub block: BlockInfo,
}

impl BlockProposal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(header: Header, txs: Vec<Transaction>, metadata: Metadata) -> Self {
        let block = BlockInfo::new(header, txs, metadata);
        Self { block }
    }
}

impl PartialEq for BlockProposal {
    fn eq(&self, other: &Self) -> bool {
        self.block.header == other.block.header && self.block.txs == other.block.txs
    }
}

impl fmt::Display for BlockProposal {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_fmt(format_args!(
            "BlockProposal {{ leader addr: {}, hash: {}, epoch: {}, slot: {}, txs: {} }}",
            self.block.metadata.address,
            self.block.header.headerhash(),
            self.block.header.epoch,
            self.block.header.slot,
            self.block.txs.len()
        ))
    }
}

impl net::Message for BlockProposal {
    fn name() -> &'static str {
        "proposal"
    }
}

impl From<BlockProposal> for BlockInfo {
    fn from(block: BlockProposal) -> BlockInfo {
        block.block
    }
}

/// This struct represents a sequence of block proposals.
#[derive(Debug, Clone, PartialEq, SerialEncodable, SerialDecodable)]
pub struct ProposalChain {
    pub genesis_block: blake3::Hash,
    pub proposals: Vec<BlockProposal>,
}

impl ProposalChain {
    pub fn new(genesis_block: blake3::Hash, initial_proposal: BlockProposal) -> Self {
        Self { genesis_block, proposals: vec![initial_proposal] }
    }

    /// A proposal is considered valid when its parent hash is equal to the
    /// hash of the previous proposal and their slots are incremental,
    /// excluding the genesis block proposal.
    /// Additional validity rules can be applied.
    pub fn check_proposal(&self, proposal: &BlockProposal, previous: &BlockProposal) -> bool {
        if proposal.block.header.previous == self.genesis_block {
            debug!("check_proposal(): Genesis block proposal provided.");
            return false
        }

        let prev_hash = previous.block.header.headerhash();
        if proposal.block.header.previous != prev_hash ||
            proposal.block.header.slot <= previous.block.header.slot
        {
            debug!("check_proposal(): Provided proposal is invalid.");
            return false
        }

        true
    }

    /// A proposals chain is considered valid when every proposal is valid,
    /// based on the `check_proposal` function.
    pub fn check_chain(&self) -> bool {
        for (index, proposal) in self.proposals[1..].iter().enumerate() {
            if !self.check_proposal(proposal, &self.proposals[index]) {
                return false
            }
        }

        true
    }

    /// Insertion of a valid proposal.
    pub fn add(&mut self, proposal: &BlockProposal) {
        if self.check_proposal(proposal, self.proposals.last().unwrap()) {
            self.proposals.push(proposal.clone());
        }
    }
}
