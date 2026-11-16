# Compila y arranca rocky en QEMU usando WHPX para acelerar la CPU invitada.
# Requiere tener habilitada la caracteristica Windows Hypervisor Platform.
cargo run --manifest-path xtask/Cargo.toml -- run-whpx
