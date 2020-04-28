use failure::Error;

use dave_torrent::app::App;


fn main() -> Result<(), Error>{
    let filename = "big-buck-bunny.torrent";
    let mut app : App = App::new();
    app.download(filename)
}
