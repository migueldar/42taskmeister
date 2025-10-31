use std::{process::Child, sync::mpsc::Receiver};

struct Jobs(Vec<Child>);

pub fn watch(rx: Receiver<Child>) {
    // let jobs: Vec<Jobs> = vec![];
    for child in rx {
        println!("My Child is: {child:#?}");
    }
}
