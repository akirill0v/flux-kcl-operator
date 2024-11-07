use flate2::read::GzDecoder;
use reqwest_middleware::ClientWithMiddleware;
use snafu::{OptionExt, ResultExt, Snafu};
use strum::{EnumDiscriminants, IntoStaticStr};

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
#[snafu(visibility(pub(crate)))]
pub enum FetcherError {
    #[snafu(display("Url parse error: {}", source))]
    InvalidParseUrl { source: url::ParseError },

    #[snafu(display("Decompress error: {}", source))]
    InvalidGzip { source: flate2::DecompressError },

    #[snafu(display("IO error: {}", source))]
    CannotCreateFile { source: std::io::Error },

    #[snafu(display("Cannot get entries: {}", source))]
    CannotGetEntries { source: std::io::Error },

    #[snafu(display("Cannot get filename"))]
    FilenameWrong,

    #[snafu(display("Cannot download: {}", source))]
    CannotDownload { source: reqwest_middleware::Error },

    #[snafu(display("Cannot get body: {}", source))]
    CannotGetBody { source: reqwest::Error },
}
