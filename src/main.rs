fn main() {
    let mut b = audio_interface::backend::create().unwrap();
    for d in b.devices().unwrap() { println!("{d:?}"); }
}
