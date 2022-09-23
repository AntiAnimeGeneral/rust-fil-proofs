use std::convert::{TryFrom, TryInto};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::mem;
use std::ops::{Index, IndexMut};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use filecoin_hashers::Hasher;
use generic_array::typenum::Unsigned;
use log::{debug, info};
use merkletree::store::{DiskStore, Store, StoreConfig};
use sha2raw::Sha256;
use storage_proofs_core::{
    drgraph::{Graph, BASE_DEGREE},
    merkle::MerkleTreeTrait,
    util::{data_at_node_offset, NODE_SIZE},
};

use crate::stacked::vanilla::{
    cache::ParentCache,
    create_label::{prepare_layers, read_layer, write_layer},
    proof::LayerState,
    Labels, LabelsCache, StackedBucketGraph,
};

#[allow(clippy::type_complexity)]
pub fn create_labels_for_encoding<Tree: 'static + MerkleTreeTrait, T: AsRef<[u8]>>(
    graph: &StackedBucketGraph<Tree::Hasher>,
    parents_cache: &mut ParentCache,
    layers: usize,
    replica_id: T,
    config: StoreConfig,
) -> Result<(Labels<Tree>, Vec<LayerState>)> {
    info!("generate labels");

    let layer_states = prepare_layers::<Tree>(graph, &config, layers);

    let layer_size = graph.size() * NODE_SIZE;
    debug!(
        "vmx: single create labels encoding: layer_size: {}",
        layer_size
    );
    // NOTE: this means we currently keep 2x sector size around, to improve speed.
    //let mut layer_labels = vec![0u8; layer_size]; // Buffer for labels of the current layer
    let mut exp_labels = vec![0u8; layer_size]; // Buffer for labels of the previous layer, needed for expander parents

    for (layer, layer_state) in (1..=layers).zip(layer_states.iter()) {
        info!("generating layer: {}", layer);
        if layer_state.generated {
            info!("skipping layer {}, already generated", layer);

            // load the already generated layer into exp_labels
            read_layer(&layer_state.config, &mut exp_labels)?;
            continue;
        }

        parents_cache.reset()?;

        let layer_path = StoreConfig::data_path(&layer_state.config.path, &layer_state.config.id);
        let mut layer_labels = NodesFile::new(&layer_path);

        if layer == 1 {
            for node in 0..graph.size() {
                create_label(
                    graph,
                    Some(parents_cache),
                    &replica_id,
                    &mut layer_labels,
                    layer,
                    node,
                )?;
            }
        } else {
            for node in 0..graph.size() {
                create_label_exp(
                    graph,
                    Some(parents_cache),
                    &replica_id,
                    &exp_labels,
                    &mut layer_labels,
                    layer,
                    node,
                )?;
            }
        }

        // Write the result to disk to avoid keeping it in memory all the time.
        let layer_config = &layer_state.config;

        //info!("  storing labels on disk");
        //write_layer(&layer_labels, layer_config).context("failed to store labels")?;
        //layer_labels.close();

        info!(
            "  generated layer {} store with id {}",
            layer, layer_config.id
        );

        // TODO vmx 2022-09-23: instead read the full layer into memory
        //info!("  setting exp parents");
        //mem::swap(&mut layer_labels, &mut exp_labels);
    }

    Ok((
        Labels::<Tree> {
            labels: layer_states.iter().map(|s| s.config.clone()).collect(),
            _h: PhantomData,
        },
        layer_states,
    ))
}

#[allow(clippy::type_complexity)]
pub fn create_labels_for_decoding<Tree: 'static + MerkleTreeTrait, T: AsRef<[u8]>>(
    graph: &StackedBucketGraph<Tree::Hasher>,
    parents_cache: &mut ParentCache,
    layers: usize,
    replica_id: T,
    config: StoreConfig,
) -> Result<LabelsCache<Tree>> {
    info!("generate labels");

    // For now, we require it due to changes in encodings structure.
    let mut labels: Vec<DiskStore<<Tree::Hasher as Hasher>::Domain>> = Vec::with_capacity(layers);

    let layer_size = graph.size() * NODE_SIZE;
    // NOTE: this means we currently keep 2x sector size around, to improve speed.
    let mut layer_labels = vec![0u8; layer_size]; // Buffer for labels of the current layer
    let mut exp_labels = vec![0u8; layer_size]; // Buffer for labels of the previous layer, needed for expander parents

    for layer in 1..=layers {
        info!("generating layer: {}", layer);

        parents_cache.reset()?;

        todo!("TODO vmx 2022-09-23: comment out for now");
        //if layer == 1 {
        //    for node in 0..graph.size() {
        //        create_label(
        //            graph,
        //            Some(parents_cache),
        //            &replica_id,
        //            &mut layer_labels,
        //            layer,
        //            node,
        //        )?;
        //    }
        //} else {
        //    for node in 0..graph.size() {
        //        create_label_exp(
        //            graph,
        //            Some(parents_cache),
        //            &replica_id,
        //            &exp_labels,
        //            &mut layer_labels,
        //            layer,
        //            node,
        //        )?;
        //    }
        //}

        // Write the result to disk to avoid keeping it in memory all the time.
        info!("  storing labels on disk");
        write_layer(&layer_labels, &config)?;

        let layer_store: DiskStore<<Tree::Hasher as Hasher>::Domain> =
            DiskStore::new_from_disk(graph.size(), Tree::Arity::to_usize(), &config)?;
        info!("  generated layer {} store with id {}", layer, config.id);

        info!("  setting exp parents");
        mem::swap(&mut layer_labels, &mut exp_labels);

        // Track the layer specific store and StoreConfig for later retrieval.
        labels.push(layer_store);
    }

    assert_eq!(
        labels.len(),
        layers,
        "Invalid amount of layers encoded expected"
    );

    Ok(LabelsCache::<Tree> { labels })
}

pub fn create_label<H: Hasher, T: AsRef<[u8]>>(
    graph: &StackedBucketGraph<H>,
    mut cache: Option<&mut ParentCache>,
    replica_id: T,
    //layer_labels: &mut [u8],
    layer_labels: &mut NodesFile,
    layer_index: usize,
    node: usize,
) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 32];

    buffer[..4].copy_from_slice(&(layer_index as u32).to_be_bytes());
    buffer[4..12].copy_from_slice(&(node as u64).to_be_bytes());
    hasher.input(&[replica_id.as_ref(), &buffer[..]][..]);

    // hash parents for all non 0 nodes
    let hash = if node > 0 {
        // prefetch previous node, which is always a parent
        //let prev = &layer_labels[(node - 1) * NODE_SIZE..node * NODE_SIZE];
        //let prev = &layer_labels[node - 1];
        //prefetch!(prev.as_ptr() as *const i8);

        //graph.copy_parents_data(node as u32, &*layer_labels, hasher, cache)?
        if let Some(ref mut cache) = cache {
            let cache_parents = cache.read(node as u32)?;

            //prefetch(&cache_parents[..BASE_DEGREE], base_data);

            // fill buffer
            let parents = [
                &layer_labels.at(cache_parents[0] as usize)[..],
                &layer_labels.at(cache_parents[1] as usize),
                &layer_labels.at(cache_parents[2] as usize),
                &layer_labels.at(cache_parents[3] as usize),
                &layer_labels.at(cache_parents[4] as usize),
                &layer_labels.at(cache_parents[5] as usize),
            ];

            // round 1 (0..6)
            hasher.input(&parents);

            // round 2 (6..12)
            hasher.input(&parents);

            // round 3 (12..18)
            hasher.input(&parents);

            // round 4 (18..24)
            hasher.input(&parents);

            // round 5 (24..30)
            hasher.input(&parents);

            // round 6 (30..36)
            hasher.input(&parents);

            // round 7 (37)
            hasher.finish_with(parents[0])
        } else {
            panic!("vmx: create labl exp: no parent cache given")
        }
    } else {
        hasher.finish()
    };

    // strip last two bits, to ensure result is in Fr.
    let mut key = hash;
    key[key.len() - 1] &= 0b0011_1111;

    // store the newly generated key
    //let start = data_at_node_offset(node);
    //let end = start + NODE_SIZE;
    //layer_labels[start..end].copy_from_slice(&hash[..]);
    layer_labels.append(&key);

    //// strip last two bits, to ensure result is in Fr.
    //layer_labels[end - 1] &= 0b0011_1111;

    Ok(())
}

#[inline]
fn prefetch(parents: &[u32], data: &[u8]) {
    for parent in parents {
        let start = *parent as usize * NODE_SIZE;
        let end = start + NODE_SIZE;

        prefetch!(data[start..end].as_ptr() as *const i8);
    }
}

#[inline]
fn read_node<'a>(i: usize, parents: &[u32], data: &'a [u8]) -> &'a [u8] {
    let start = parents[i] as usize * NODE_SIZE;
    let end = start + NODE_SIZE;
    &data[start..end]
}

pub fn create_label_exp<H: Hasher, T: AsRef<[u8]>>(
    graph: &StackedBucketGraph<H>,
    mut cache: Option<&mut ParentCache>,
    replica_id: T,
    exp_parents_data: &[u8],
    //layer_labels: &mut [u8],
    layer_labels: &mut NodesFile,
    layer_index: usize,
    node: usize,
) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 32];

    buffer[0..4].copy_from_slice(&(layer_index as u32).to_be_bytes());
    buffer[4..12].copy_from_slice(&(node as u64).to_be_bytes());
    hasher.input(&[replica_id.as_ref(), &buffer[..]][..]);

    // hash parents for all non 0 nodes
    let hash = if node > 0 {
        //// prefetch previous node, which is always a parent
        //let prev = &layer_labels[(node - 1) * NODE_SIZE..node * NODE_SIZE];
        //prefetch!(prev.as_ptr() as *const i8);

        //graph.copy_parents_data_exp(node as u32, &*layer_labels, exp_parents_data, hasher, cache)?
        if let Some(ref mut cache) = cache {
            let cache_parents = cache.read(node as u32)?;
            //Ok(self.copy_parents_data_inner_exp(&cache_parents, base_data, exp_data, hasher))

            // TODO vmx 2022-09-23: check if prefetching makes a difference
            //prefetch(&cache_parents[..BASE_DEGREE], base_data);
            prefetch(&cache_parents[BASE_DEGREE..], exp_parents_data);

            // fill buffer
            let parents = [
                //read_node(0, cache_parents, base_data),
                //read_node(1, cache_parents, base_data),
                //read_node(2, cache_parents, base_data),
                //read_node(3, cache_parents, base_data),
                //read_node(4, cache_parents, base_data),
                //read_node(5, cache_parents, base_data),
                &layer_labels.at(cache_parents[0] as usize),
                &layer_labels.at(cache_parents[1] as usize),
                &layer_labels.at(cache_parents[2] as usize),
                &layer_labels.at(cache_parents[3] as usize),
                &layer_labels.at(cache_parents[4] as usize),
                &layer_labels.at(cache_parents[5] as usize),
                read_node(6, &cache_parents, &exp_parents_data),
                read_node(7, &cache_parents, &exp_parents_data),
                read_node(8, &cache_parents, &exp_parents_data),
                read_node(9, &cache_parents, &exp_parents_data),
                read_node(10, &cache_parents, &exp_parents_data),
                read_node(11, &cache_parents, &exp_parents_data),
                read_node(12, &cache_parents, &exp_parents_data),
                read_node(13, &cache_parents, &exp_parents_data),
            ];

            // round 1 (14)
            hasher.input(&parents);

            // round 2 (14)
            hasher.input(&parents);

            // round 3 (9)
            hasher.input(&parents[..8]);
            hasher.finish_with(parents[8])
        } else {
            panic!("vmx: create labl exp: no parent cache given")
        }
    } else {
        hasher.finish()
    };

    // strip last two bits, to ensure result is in Fr.
    let mut key = hash;
    key[key.len() - 1] &= 0b0011_1111;

    // store the newly generated key
    //let start = data_at_node_offset(node);
    //let end = start + NODE_SIZE;
    //layer_labels[start..end].copy_from_slice(&hash[..]);
    layer_labels.append(&key);

    //// strip last two bits, to ensure result is in Fr.
    //layer_labels[end - 1] &= 0b0011_1111;

    Ok(())
}

const WRITE_BUFFER_SIZE: usize = 128 * 1024 * 1024;

/// A wrapper around a file to read out nodes by giving an index.
pub struct NodesFile {
    //reader: BufReader<File>,
    reader: File,
    //writer: BufWriter<File>,
    writer: File,
    /// The offset up to which the data is already written to disk.
    written_offset: usize,
    /// The buffer that is filled before writing to disk. This reduces `write` syscalls.
    write_buffer: Vec<u8>,
}

impl NodesFile {
    pub fn new(path: &PathBuf) -> Self {
        let writer = File::options()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        //let reader = BufReader::new(File::open(path).unwrap());
        let reader = File::open(path).unwrap();
        assert_eq!(
            WRITE_BUFFER_SIZE % NODE_SIZE,
            0,
            "Full nodes can be written into write buffer."
        );
        Self {
            reader,
            //writer: BufWriter::with_capacity(16777216, writer),
            writer,
            written_offset: 0,
            write_buffer: Vec::with_capacity(WRITE_BUFFER_SIZE),
        }
    }

    fn at(&mut self, node: usize) -> [u8; 32] {
        let pos = node * NODE_SIZE;
        if pos < self.written_offset {
            let mut buf = [0u8; 32];
            self.reader
                .seek(SeekFrom::Start(u64::try_from(pos).unwrap()))
                .unwrap();
            self.reader.read_exact(&mut buf).unwrap();
            buf
        } else {
            let buffer_pos = pos - self.written_offset;
            self.write_buffer[buffer_pos..buffer_pos + NODE_SIZE]
                .try_into()
                .unwrap()
        }
    }

    /// Appends a node to the end of the file
    pub fn append(&mut self, node: &[u8; 32]) {
        self.write_buffer.extend_from_slice(node);
        if self.write_buffer.len() == WRITE_BUFFER_SIZE {
            self.writer.write_all(&self.write_buffer).unwrap();
            // TODO vmx 2022-09-24: Check if there's a faster way than using `clear()`.
            self.write_buffer.clear();
            self.written_offset += WRITE_BUFFER_SIZE;
        }
    }
}

//impl Index<usize> for NodesFile {
//    type Output = [u8];
//    fn index(&self, node: usize) -> &Self::Output {
//        let mut buf = [0u8; 32];
//        let pos = node * NODE_SIZE;
//        self.file.seek(SeekFrom::Start(u64::try_from(pos).unwrap())).unwrap();
//        self.file.read_exact(&mut buf).unwrap();
//        &buf
//    }
//}
