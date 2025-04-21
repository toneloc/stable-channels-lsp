pub mod price_feeds;
pub mod types;
pub mod stable;

#[cfg(feature = "user")]
mod user;

#[cfg(any(feature = "lsp", feature = "exchange"))]
mod server;

#[cfg(all(feature = "user", not(any(feature = "lsp", feature = "exchange"))))]
fn main() {
    user::run();
}

#[cfg(all(not(feature = "user"), any(feature = "lsp", feature = "exchange")))]
fn main() {
    let mode = if cfg!(feature = "lsp") {
        "lsp"
    } else {
        "exchange"
    };

    server::run_with_mode(mode);
}

#[cfg(not(any(feature = "user", feature = "lsp", feature = "exchange")))]
fn main() {
    panic!("Must compile with --features user, lsp, or exchange");
}