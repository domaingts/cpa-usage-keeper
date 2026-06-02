/// Application version. Override at build time via:
///   cargo rustc --release -- --cfg "build_version=\"v1.6.0\""
/// or set the `CPA_USAGE_KEEPER_VERSION` env var when running.
pub const VERSION: &str = match option_env!("CPA_USAGE_KEEPER_VERSION") {
    Some(v) => v,
    None => "dev",
};
