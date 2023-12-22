use anyhow::Result;
use fil_proofs_bin::cli;
use filecoin_hashers::sha256::Sha256Domain;
use log::info;
use serde::{Deserialize, Serialize};
use serde_hex::{SerHex, StrictPfx};
use storage_proofs_core::util::NODE_SIZE;
use storage_proofs_porep::stacked::NiChallenges;

#[derive(Debug, Deserialize, Serialize)]
struct ChallengesNiParameters {
    #[serde(with = "SerHex::<StrictPfx>")]
    comm_r: [u8; 32],
    num_challenges_per_partition: usize,
    num_partitions: usize,
    #[serde(with = "SerHex::<StrictPfx>")]
    replica_id: [u8; 32],
    /// Sector size is used to calculate the number of nodes.
    sector_size: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChallengesNiOutput {
    challenges: Vec<usize>,
}

fn main() -> Result<()> {
    fil_logger::maybe_init();

    let params: ChallengesNiParameters = cli::parse_stdin()?;
    info!("{:?}", params);

    let challenges = NiChallenges::new(params.num_challenges_per_partition);
    let sector_nodes = usize::try_from(params.sector_size)
        .expect("sector size must be smaller than the default integer size on this platform")
        / NODE_SIZE;

    let challenge_positions = (0..params.num_partitions)
        .flat_map(|k| {
            challenges.derive::<Sha256Domain>(
                sector_nodes,
                &params.replica_id.into(),
                &params.comm_r.into(),
                k as u8,
            )
        })
        .collect::<Vec<usize>>();

    let output = ChallengesNiOutput {
        challenges: challenge_positions,
    };
    info!("{:?}", output);
    cli::print_stdout(output)?;

    Ok(())
}