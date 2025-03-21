pub mod price_feeds;
pub mod types;

#[cfg(feature = "user")]
mod user;

#[cfg(feature = "lsp")]
mod lsp;

#[cfg(feature = "exchange")]
mod exchange;

fn main() {
    #[cfg(feature = "user")]
    {
        println!("Starting in User mode");
        user::run();
    }
    
    #[cfg(feature = "lsp")]
    {
        println!("Starting in LSP mode");
        lsp::run();
    }

    #[cfg(feature = "exchange")]
    {
        println!("Starting in Exchange mode");
        exchange::run();
    }
    
    #[cfg(not(any(feature = "exchange", feature = "user", feature = "lsp")))]
    {
        println!("Error: No component selected.");
        println!("Please build with one of the following features:");
        println!("  --features exchange");
        println!("  --features user");
        println!("  --features lsp");
    }
}