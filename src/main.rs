// unused allowed: the CLI dispatch that calls into this module lands in a
// later migration branch (arg parsing + command wiring).
#[allow(unused)]
mod cli;
#[allow(unused)]
mod config;
#[allow(unused)]
mod exec;

fn main() {
    println!("Hello, world!");
}
