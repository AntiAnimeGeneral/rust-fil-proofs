use std::{
    fs::{self, DirEntry},
    path::PathBuf,
};

use anyhow::bail;
use byte_unit::Byte;
use clap::Parser;
use fil_proofs_tooling::Metadata;
use filecoin_proofs::*;
use rayon::iter::{
    IntoParallelIterator, IntoParallelRefIterator, ParallelIterator,
};
use serde_json::to_string_pretty;
use storage_proofs_core::api_version::{ApiFeature, ApiVersion};

mod porep;
mod window_post;

#[derive(Debug, Parser)]
enum Cli {
    WindowPost(WindowPost),
    Seal(Seal),
}

#[derive(Debug, Parser)]
struct WindowPost {
    #[clap(long)]
    /// Traveling the dir
    walkdir: Option<PathBuf>,

    #[clap(long)]
    /// The file dir
    dir: Option<PathBuf>,

    #[clap(long, default_value_t = ApiVersion::V1_2_0)]
    api_version: ApiVersion,
}

#[derive(Debug, Parser)]
struct Seal {
    /// Size of sector
    #[clap(long,value_parser=parse_size)]
    size: usize,

    /// The cache file dir
    #[clap(long)]
    cache: PathBuf,

    /// Num of tasks
    #[clap(long, default_value_t = 1)]
    tasks: usize,

    #[clap(long, default_value_t = ApiVersion::V1_2_0)]
    api_version: ApiVersion,
    api_features: Vec<ApiFeature>,

    #[clap(long, default_value_t = true)]
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

fn main() -> anyhow::Result<()> {
    fil_logger::init();

    let cli = Cli::parse();
    match cli {
        Cli::Seal(Seal {
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
        }) => {
            (0..tasks).into_par_iter().for_each(|task| {
                let cache = cache.join(format!("task-{task}"));
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
        Cli::WindowPost(WindowPost {
            walkdir,
            dir,
            api_version,
        }) => match (dir, walkdir) {
            (Some(dir), None) => {
                let rep = window_post::run(api_version, dir)?;
                let wrapped = Metadata::wrap(&rep)
                    .expect("failed to retrieve metadata");
                let js = to_string_pretty(&wrapped)?;
                println!("report: {js}")
            }
            (None, Some(walkdir)) => {
                let dir = fs::read_dir(walkdir)?
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?;
                dir.par_iter()
                    .filter(|path| path.path().is_dir())
                    .map(DirEntry::path)
                    .for_each(|dir| {
                        let rep = window_post::run(api_version, dir)
                            .unwrap();
                        let wrapped = Metadata::wrap(&rep)
                            .expect("failed to retrieve metadata");
                        let js = to_string_pretty(&wrapped).unwrap();
                        println!("report: {js}")
                    });
            }
            _ => {
                bail!("")
            }
        },
    };
    Ok(())
}

#[cfg(test)]
mod debug {
    use std::path::Path;

    use crate::porep::{self, Report};
    use storage_proofs_core::api_version::ApiVersion;

    #[test]
    fn test2k() {
        fil_logger::init();
        run(2 << 10, "./cache/test2k").unwrap();
    }

    #[test]
    fn test4k() {
        fil_logger::init();
        run(4 << 10, "./cache/test4k").unwrap();
    }

    #[test]
    fn test8m() {
        fil_logger::init();
        run(8 << 20, "./cache/test8m").unwrap();
    }

    #[test]
    fn test16m() {
        fil_logger::init();
        run(16 << 20, "./cache/test16m").unwrap();
    }

    fn run(
        sector_size: usize,
        cache_dir: impl AsRef<Path>,
    ) -> anyhow::Result<Report> {
        porep::run(
            sector_size,
            ApiVersion::V1_2_0,
            vec![],
            cache_dir.as_ref().to_path_buf(),
            true,
            false,
            false,
            false,
            false,
            false,
        )
    }
}
