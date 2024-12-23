use std::collections::BTreeMap;
use std::fs::read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use bincode::deserialize;
use fil_proofs_tooling::measure;
use fil_proofs_tooling::shared::{PROVER_ID, RANDOMNESS};
use filecoin_proofs::constants::{
    WINDOW_POST_CHALLENGE_COUNT, WINDOW_POST_SECTOR_COUNT,
};
use filecoin_proofs::types::{
    PoStConfig, SealPreCommitOutput, SectorSize,
};
use filecoin_proofs::{
    generate_window_post, verify_window_post, with_shape, PoStType,
    PrivateReplicaInfo, PublicReplicaInfo,
};
use log::info;
use serde::{Deserialize, Serialize};
use storage_proofs_core::{
    api_version::ApiVersion, merkle::MerkleTreeTrait,
    sector::SectorId,
};
const SECTOR_ID: u64 = 0;

const SEALED_FILE: &str = "sealed-file";
const PRECOMMIT_PHASE2_OUTPUT_FILE: &str = "precommit-phase2-output";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Inputs {
    sector_size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Outputs {
    gen_window_post_cpu_time_ms: u64,
    gen_window_post_wall_time_ms: u64,
    verify_window_post_cpu_time_ms: u64,
    verify_window_post_wall_time_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Report {
    inputs: Inputs,
    outputs: Outputs,
}

#[allow(clippy::too_many_arguments)]
pub fn run_window_post_bench<Tree: 'static + MerkleTreeTrait>(
    sector_size: u64,
    api_version: ApiVersion,
    cache_dir: PathBuf,
) -> anyhow::Result<Report> {
    let seal_pre_commit_output = {
        let phase2_output_path =
            cache_dir.join(PRECOMMIT_PHASE2_OUTPUT_FILE);
        info!("*** Restoring precommit phase2 output file");
        let phase2_output_bytes = read(&phase2_output_path)
            .with_context(|| {
                format!(
                    "could not read file phase2_output_path={:?}",
                    phase2_output_path
                )
            })?;

        let res: SealPreCommitOutput =
            deserialize(&phase2_output_bytes)?;

        res
    };

    let comm_r = seal_pre_commit_output.comm_r;

    let sector_id = SectorId::from(SECTOR_ID);

    let sealed_file_path = cache_dir.join(SEALED_FILE);

    let pub_replica = PublicReplicaInfo::new(comm_r)
        .expect("failed to create public replica info");

    let priv_replica = PrivateReplicaInfo::<Tree>::new(
        sealed_file_path,
        comm_r,
        cache_dir.clone(),
    )
    .expect("failed to create private replica info");

    // Store the replica's private and publicly facing info for proving and verifying respectively.
    let mut pub_replica_info: BTreeMap<SectorId, PublicReplicaInfo> =
        BTreeMap::new();
    let mut priv_replica_info: BTreeMap<
        SectorId,
        PrivateReplicaInfo<Tree>,
    > = BTreeMap::new();

    pub_replica_info.insert(sector_id, pub_replica);
    priv_replica_info.insert(sector_id, priv_replica);

    // Measure PoSt generation and verification.
    let post_config = PoStConfig {
        sector_size: SectorSize(sector_size),
        challenge_count: WINDOW_POST_CHALLENGE_COUNT,
        sector_count: *WINDOW_POST_SECTOR_COUNT
            .read()
            .expect("WINDOW_POST_SECTOR_COUNT poisoned")
            .get(&sector_size)
            .expect("unknown sector size"),
        typ: PoStType::Window,
        priority: true,
        api_version,
    };

    let gen_window_post_measurement = measure(|| {
        generate_window_post::<Tree>(
            &post_config,
            &RANDOMNESS,
            &priv_replica_info,
            PROVER_ID,
        )
    })
    .expect("failed to generate window post");

    let proof = &gen_window_post_measurement.return_value;

    // warmup cache
    verify_window_post::<Tree>(
        &post_config,
        &RANDOMNESS,
        &pub_replica_info,
        PROVER_ID,
        proof,
    )
    .unwrap();
    let verify_window_post_measurement = measure(|| {
        verify_window_post::<Tree>(
            &post_config,
            &RANDOMNESS,
            &pub_replica_info,
            PROVER_ID,
            proof,
        )
    })
    .expect("failed to verify window post proof");

    let report = Report {
        inputs: Inputs { sector_size },
        outputs: Outputs {
            gen_window_post_cpu_time_ms: gen_window_post_measurement
                .cpu_time
                .as_millis()
                as u64,
            gen_window_post_wall_time_ms: gen_window_post_measurement
                .wall_time
                .as_millis()
                as u64,
            verify_window_post_cpu_time_ms:
                verify_window_post_measurement.cpu_time.as_millis()
                    as u64,
            verify_window_post_wall_time_ms:
                verify_window_post_measurement.wall_time.as_millis()
                    as u64,
        },
    };

    Ok(report)
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    api_version: ApiVersion,
    dir: impl AsRef<Path>,
) -> anyhow::Result<Report> {
    let dir = dir.as_ref();

    let seal_path = dir.join(SEALED_FILE);
    let info = std::fs::metadata(seal_path)?;
    let sector_size = info.size();

    with_shape!(
        sector_size as u64,
        run_window_post_bench,
        sector_size as u64,
        api_version,
        dir.to_path_buf(),
    )
}
