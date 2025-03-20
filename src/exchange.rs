#[cfg(feature = "exchange")]
use std::time::Duration;
use std::thread;

#[cfg(feature = "exchange")]
pub fn run() {
    println!("Exchange component started");
  
    thread::sleep(Duration::from_secs(3));
    println!("Exchange component finished");
}