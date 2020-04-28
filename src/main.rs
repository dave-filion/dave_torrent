use failure::Error;

use dave_torrent::app::App;


fn main() -> Result<(), Error>{
    let filename = "manjaro-kde-20.0-200426-linux56.iso.torrent";
    let mut app : App = App::new();
    app.download(filename)
}
