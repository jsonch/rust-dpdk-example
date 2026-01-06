# Rust DPDK Reflector Example

A Rust implementation of a simple DPDK packet reflector that receives packets on a port and sends them back out the same port.

## Dependencies

Build and install the rust dpdk bindings using the `dpdk-install.sh` script from here: https://github.com/jsonch/rust-dpdk


## Building

```bash
cargo build --release
```

## Running with dpdk pcap IO

```bash
# Create a test pcap file (you can use your own)
# Run the reflector
sudo target/release/reflector -l 0 --no-huge --no-pci --vdev 'net_pcap0,rx_pcap=test.pcap,tx_pcap=out.pcap' -- 0
```

## Running with veth pairs and dpdk af_xdp IO

Start up a test veth interface:

```bash
sudo ./veth-up.sh
```
Run on the receiver end of the interface:
```bash
sudo target/release/reflector -l 0 --no-huge --no-pci   --vdev='net_af_xdp0,iface=receiver,force_copy=1' -- 0
```

Send the pcap on the sender end:

```bash
sudo tcpreplay -i sender --pps=10 test.pcap
```



## Project Structure

- `src/reflector.rs` - Main reflector implementation
- `Cargo.toml` - Rust project configuration

This example demonstrates:
- DPDK EAL initialization from Rust
- Port configuration and setup
- Mbuf pool creation
- RX/TX queue setup
- Packet receive and transmit burst operations
- Memory management with mbufs
