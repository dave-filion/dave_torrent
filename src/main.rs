use failure::Error;

use dave_torrent::app::App;


fn main() -> Result<(), Error>{
    let args: Vec<String> = std::env::args().collect();
    let filename = if let Some(f) = args.get(1) {
        f
    } else {
        panic!("No filename argument!");
    };

    let mut app : App = App::new();
    app.download(filename)
}
