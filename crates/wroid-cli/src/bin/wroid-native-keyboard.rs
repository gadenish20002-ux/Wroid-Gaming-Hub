use std::error::Error;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    wroid_inject::run_live_keyboard_cli(&args, "wroid-native-keyboard")
}
