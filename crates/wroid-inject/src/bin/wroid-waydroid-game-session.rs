fn main() -> wroid_inject::GameSessionResult<()> {
    wroid_inject::run_game_session_cli(std::env::args().skip(1))
}
