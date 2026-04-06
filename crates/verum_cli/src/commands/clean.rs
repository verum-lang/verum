// Clean command - remove build artifacts

use crate::config::Manifest;
use crate::error::Result;
use crate::ui;
use std::fs;

pub fn execute(all: bool) -> Result<()> {
    ui::step("Cleaning build artifacts");

    let manifest_dir = Manifest::find_manifest_dir()?;
    let target_dir = manifest_dir.join("target");

    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
        ui::success("Removed target directory");
    }

    if all {
        let cache_file = manifest_dir.join(".verum_cache");
        if cache_file.exists() {
            fs::remove_file(&cache_file)?;
            ui::success("Removed cache file");
        }
    }

    Ok(())
}
