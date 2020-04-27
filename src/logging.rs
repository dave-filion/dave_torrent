
// TODO have this be an env variable
const DEBUG_MODE: bool = false;

pub fn debug(s: String) {
    if DEBUG_MODE {
        println!("{}", s);
    }
}