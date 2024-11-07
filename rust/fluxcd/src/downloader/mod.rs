use std::{
    fs::{create_dir_all, File},
    io::Cursor,
    path::PathBuf,
};

use crate::downloader::error::*;
use flate2::read::GzDecoder;
use reqwest_middleware::ClientWithMiddleware;
use snafu::{OptionExt, ResultExt};
use tar::Archive;
use tracing::info;
use url::Url;

pub mod error;

type Result<T, E = DownloaderError> = std::result::Result<T, E>;

pub struct Downloader {
    client: ClientWithMiddleware,
    host: Option<String>,

    storage_dir: PathBuf,
}

impl Downloader {
    pub fn new(
        client: ClientWithMiddleware,
        host: Option<String>,
        storage_dir: Option<PathBuf>,
    ) -> Self {
        let storage_dir = storage_dir.unwrap_or_else(|| PathBuf::from("/tmp/kcl"));
        Self {
            client,
            host,
            storage_dir,
        }
    }

    /// # Type: Directory downloader
    ///
    /// Helper to download files over http into a directory
    ///
    /// # Example:
    /// ```ignore
    /// let file_downloader = Downloader::new(client, host);
    /// let path = file_downloader.download("http://example.com/file.tar.gz", "my-repo", dest_path).await?;
    /// ```
    ///
    /// # Errors:
    /// Returns a DownloaderError in the following cases:
    /// - If the file cannot be downloaded
    /// - If the file cannot be written to disk
    /// - If the tar.gz file cannot be extracted
    /// - If the URL is invalid
    ///
    pub async fn download(&self, url: &str, repo_name: &str, namespace: &str) -> Result<PathBuf> {
        let url = build_url(url, self.host.clone())?;
        let path = self.storage_dir.join(namespace).join(repo_name);

        let target = url
            .path_segments()
            .and_then(|segments| segments.last())
            .context(FilenameWrongSnafu)?;

        let target_path = path.join(target);

        // Create the directory if it doesn't exist
        if !path.exists() {
            info!("Creating directory {}", path.display());
            create_dir_all(&path).context(CannotCreateFileSnafu)?;
        }

        //  Check if the file already exists and download it if not
        if !target_path.exists() {
            info!("Downloading stream from {}", url);
            let response = self
                .client
                .get(url.clone())
                .send()
                .await
                .context(CannotDownloadSnafu)?;

            // Open a file to write the downloaded content
            let mut file = File::create(&target_path).context(CannotCreateFileSnafu)?;
            // Copy the content from the response to the file
            let mut content = Cursor::new(response.bytes().await.context(CannotGetBodySnafu)?);
            std::io::copy(&mut content, &mut file).context(CannotCreateFileSnafu)?;
        }

        // dir_path is the name of file without the extension
        // Check if the directory exists
        let dir_path = path.join(target.trim_end_matches(".tar.gz"));
        if !dir_path.exists() {
            // Extract the tar.gz file to the target directory
            info!("Extracting file to {}", &dir_path.display());
            let tar_gz = File::open(&target_path).context(CannotCreateFileSnafu)?;
            let mut archive = Archive::new(GzDecoder::new(tar_gz));
            archive.unpack(&dir_path).context(CannotCreateFileSnafu)?;
            info!("Extracted file to {}", &dir_path.display());
        }

        Ok(dir_path)
    }
}

pub(crate) fn build_url(url: &str, override_host: Option<String>) -> Result<Url> {
    tracing::info!(
        "Building url {} with override host {}",
        url,
        override_host.as_deref().unwrap_or("None")
    );
    let parsed_url = url::Url::parse(url).context(InvalidParseUrlSnafu)?;
    if let Some(host) = override_host {
        let mut override_parsed = url::Url::parse(host.as_str()).context(InvalidParseUrlSnafu)?;
        override_parsed.set_path(parsed_url.path());
        override_parsed.set_query(parsed_url.query());
        Ok(override_parsed)
    } else {
        Ok(parsed_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_url_no_override() -> Result<()> {
        let url = "http://example.com/path";
        let result = build_url(url, None)?;
        assert_eq!(result.to_string(), url);
        Ok(())
    }

    #[test]
    fn test_build_url_with_override() -> Result<()> {
        let url = "http://source-controller.flux-system.svc.cluster.local./gitrepository/flux-system/podinfo/6b7aab8a10d6ee8b895b0a5048f4ab0966ed29ff.tar.gz";
        let override_host = Some("http://127.0.0.1:8080".to_string());
        let result = build_url(url, override_host)?;
        assert_eq!(result.to_string(), "http://127.0.0.1:8080/gitrepository/flux-system/podinfo/6b7aab8a10d6ee8b895b0a5048f4ab0966ed29ff.tar.gz");
        Ok(())
    }

    #[test]
    fn test_build_url_invalid_url() {
        let url = "not a url";
        let result = build_url(url, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_url_invalid_override() {
        let url = "http://example.com/path";
        let override_host = Some("not a url".to_string());
        let result = build_url(url, override_host);
        assert!(result.is_err());
    }
}
