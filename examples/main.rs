extern crate nix;
extern crate tokio;
extern crate axon;

use std::env;
use std::process::Command;
use std::io::{Read,Write};

use axon::CommandExt;

fn main() {
    if env::args().len() > 1 {
        child();
        return;
    }
    let (mut child, mut io) = Command::new(env::current_exe().unwrap())
        .arg("child")
        .spawn_with_axon()
        .expect("Failed to start echo process");

    io.write("yolo".as_bytes()).unwrap();

    let mut b = vec![0;20];
    io.read(&mut b).unwrap();
    println!("recv in parent: {:?}", b);

    let ecode = child.wait().expect("failed to wait on child");
    assert!(ecode.success());
}

fn child() {
    let mut io = axon::child().expect("axiom setup");
    let mut b = vec![0;10];
    io.read(&mut b).expect("reading from axiom file descriptor");
    io.write("ok got it".as_bytes()).expect("sending on axiom failed");
    println!("recv in child: {:?}", b);
}
