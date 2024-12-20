use anyhow::bail;
use byte_unit::Byte;
use clap::Parser;
use fil_proofs_tooling::Metadata;
use filecoin_proofs::*;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde_json::to_string_pretty;
use storage_proofs_core::api_version::{ApiFeature, ApiVersion};

mod porep;

#[derive(Debug, Parser)]
struct Cli {
    /// Size of sector
    #[clap(long,value_parser=parse_size)]
    size: usize,

    #[clap(long)]
    /// The cache file dir
    cache: String,

    /// Num of tasks
    #[clap(long, default_value_t = 1)]
    tasks: usize,

    #[clap(long, default_value_t = ApiVersion::V1_2_0)]
    api_version: ApiVersion,
    api_features: Vec<ApiFeature>,
    preserve_cache: bool,
    skip_precommit_phase1: bool,
    skip_precommit_phase2: bool,
    skip_commit_phase1: bool,
    skip_commit_phase2: bool,
    test_resume: bool,
}

fn parse_size(size: &str) -> anyhow::Result<usize> {
    let size = Byte::from_str(size)?.get_bytes() as usize;
    match size as u64 {
        SECTOR_SIZE_2_KIB | SECTOR_SIZE_4_KIB
        | SECTOR_SIZE_16_KIB | SECTOR_SIZE_32_KIB
        | SECTOR_SIZE_8_MIB | SECTOR_SIZE_16_MIB
        | SECTOR_SIZE_512_MIB | SECTOR_SIZE_1_GIB
        | SECTOR_SIZE_32_GIB | SECTOR_SIZE_64_GIB => Ok(size),
        _ => bail!("unsupported sector size: {size}"),
    }
}

fn main() {
    fil_logger::init();

    let Cli {
        size,
        cache,
        tasks,
        api_version,
        api_features,
        preserve_cache,
        skip_precommit_phase1,
        skip_precommit_phase2,
        skip_commit_phase1,
        skip_commit_phase2,
        test_resume,
    } = Cli::parse();

    if tasks == 1 {
        let rep = porep::run(
            size,
            api_version,
            api_features,
            cache,
            preserve_cache,
            skip_precommit_phase1,
            skip_precommit_phase2,
            skip_commit_phase1,
            skip_commit_phase2,
            test_resume,
        )
        .unwrap();
        let wrapped = Metadata::wrap(&rep)
            .expect("failed to retrieve metadata");
        let js = to_string_pretty(&wrapped).unwrap();
        println!("report: {js}")
    } else {
        (0..tasks).into_par_iter().for_each(|task| {
            let mut cache = cache.clone();
            cache.push_str(&format!("/task-{task}"));
            let rep = porep::run(
                size,
                api_version,
                api_features.clone(),
                cache,
                preserve_cache,
                skip_precommit_phase1,
                skip_precommit_phase2,
                skip_commit_phase1,
                skip_commit_phase2,
                test_resume,
            )
            .unwrap();
            let wrapped = Metadata::wrap(&rep)
                .expect("failed to retrieve metadata");
            let js = to_string_pretty(&wrapped).unwrap();
            println!("task-{task}-report: {js}")
        });
    }
}

#[cfg(test)]
mod debug {
    use crate::porep::{self, Report};
    use storage_proofs_core::api_version::ApiVersion;

    #[test]
    fn test2k() {
        fil_logger::init();
        run(2 << 10, "./cache/test2k".into()).unwrap();
    }

    #[test]
    fn test4k() {
        fil_logger::init();
        run(4 << 10, "./cache/test4k".into()).unwrap();
    }

    #[test]
    fn test8m() {
        fil_logger::init();
        run(8 << 20, "./cache/test8m".into()).unwrap();
    }

    #[test]
    fn test16m() {
        fil_logger::init();
        run(16 << 20, "./cache/test16m".into()).unwrap();
    }

    fn run(
        sector_size: usize,
        cache_dir: String,
    ) -> anyhow::Result<Report> {
        porep::run(
            sector_size,
            ApiVersion::V1_2_0,
            vec![],
            cache_dir,
            true,
            false,
            false,
            false,
            false,
            false,
        )
    }
}
