use crate::{types::Slug, utils::shim::write_shim_to_file};

pub fn add(command: Slug) -> eyre::Result<()> {
    write_shim_to_file(&command)
}
