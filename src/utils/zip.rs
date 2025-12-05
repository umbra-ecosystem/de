use eyre::{WrapErr, eyre};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use zip::write::SimpleFileOptions;

pub fn zip_dir(zip_file: File, dir: &Path) -> eyre::Result<()> {
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);

    let prefix = Path::new(dir);
    let mut buffer = Vec::new();

    for entry in walkdir::WalkDir::new(dir).max_depth(10) {
        let entry = entry
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to read directory entry in: {}", dir.display()))?;

        let path = entry.path();
        let name = path.strip_prefix(prefix).unwrap();
        let path_as_string = name
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| eyre!("{name:?} Is a Non UTF-8 Path"))?;

        if path.is_file() {
            tracing::debug!("Adding file to zip: {path_as_string}");
            zip.start_file(path_as_string, options)?;
            let mut f = File::open(path)?;
            f.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
            buffer.clear();
        } else if !name.as_os_str().is_empty() {
            tracing::debug!("Adding dir to zip: {path_as_string}");
            zip.add_directory(path_as_string, options)?;
        }
    }

    zip.finish()?;

    Ok(())
}

pub fn extract_zip(zip_file: File, target_dir: &Path) -> eyre::Result<()> {
    let mut archive = zip::ZipArchive::new(zip_file)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to read zip archive")?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to access file at index {i} in zip archive"))?;

        let outpath = if let Some(name) = file.enclosed_name() {
            target_dir.join(name)
        } else {
            return Err(eyre!(
                "Invalid or potentially unsafe file name in zip archive: '{}'. \
                This may indicate a path traversal attempt or a corrupted archive.",
                file.name()
            ));
        };

        if file.is_dir() {
            tracing::debug!("Creating directory: {}", outpath.display());

            std::fs::create_dir_all(&outpath)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to create directory: {}", outpath.display()))?;
        } else {
            tracing::debug!("Extracting file to: {}", outpath.display());

            if let Some(parent) = outpath.parent()
                && !parent.exists()
            {
                std::fs::create_dir_all(parent)
                    .map_err(|e| eyre!(e))
                    .wrap_err_with(|| {
                        format!("Failed to create directory: {}", parent.display())
                    })?;
            }

            let mut outfile = File::create(&outpath)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to create file: {}", outpath.display()))?;

            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to write to file: {}", outpath.display()))?;
        }
    }

    Ok(())
}
