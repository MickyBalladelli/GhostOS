use synos_vm::Vm;

fn main() {
    println!("SynOS Virtual Machine");
    println!("=====================");
    
    let mut vm = Vm::new();
    
    if let Err(e) = vm.run() {
        eprintln!("VM Error: {:?}", e);
    }
}