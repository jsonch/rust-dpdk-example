// Rust implementation of reflector.c - a simple DPDK packet reflector
// This program receives packets on a port and sends them back out the same port

use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::alloc::{alloc, Layout};
use std::mem::size_of;

// Import DPDK bindings
use dpdk_sys::*;

const RING_SIZE: u16 = 2048;
const NUM_MBUFS: u32 = 8192;
const MBUF_CACHE_SIZE: u32 = 250;
const MAX_PKT_BURST: u16 = 4;

unsafe fn port_init(port: u16) -> Result<(), i32> {
    if rte_eth_dev_is_valid_port(port) == 0 {
        return Err(-1);
    }

    // --- MANUAL MEMORY ALLOCATION START ---
    let pool_name = CString::new(format!("MBUF_POOL_{}", port)).unwrap();
    
    // CHANGE 1: Use 2048 byte elements.
    // This allows 2 objects to fit perfectly in one 4KB page with NO padding.
    // It creates a standard "2K" stride which AF_XDP loves.
    // Usable data room will be: 2048 - 128 (mbuf) - 128 (headroom) = 1792 bytes.
    // This is plenty for standard MTU (1500).
    let elt_size = 2048; 
    
    // CHANGE 2: Calculate total memory (N * 2048)
    let total_mem_size = (NUM_MBUFS as usize * elt_size) + 4096;

    // 3. Keep Force 4KB (page) alignment for the Base Address
    let layout = Layout::from_size_align(total_mem_size, 4096).unwrap();
    let raw_mem = alloc(layout);

    if raw_mem.is_null() {
        eprintln!("Failed to allocate page-aligned memory");
        return Err(-1);
    }

    // 4. Create an EMPTY mempool
    let mbuf_pool = rte_mempool_create_empty(
        pool_name.as_ptr(),
        NUM_MBUFS,
        elt_size as u32,
        MBUF_CACHE_SIZE,
        size_of::<rte_pktmbuf_pool_private>() as u32,
        rte_socket_id() as i32,
        0,
    );

    if mbuf_pool.is_null() {
        eprintln!("Cannot create empty mempool");
        return Err(-1);
    }

    // 5. Set the handlers to Ring
    let ring_ops = CString::new("ring_mp_mc").unwrap();
    rte_mempool_set_ops_byname(mbuf_pool, ring_ops.as_ptr(), ptr::null_mut());

    // 6. Populate the pool
    // DPDK sees 2048 fits twice into 4096. It will pack them tightly.
    let ret = rte_mempool_populate_virt(
        mbuf_pool,
        raw_mem as *mut _,
        total_mem_size,
        4096, 
        None, 
        ptr::null_mut(),
    );

    if ret < 0 {
        eprintln!("Error populating mempool: {}", ret);
        return Err(ret);
    }

    // 7. Initialize the mbuf headers
    rte_pktmbuf_pool_init(mbuf_pool, ptr::null_mut());
    rte_mempool_obj_iter(mbuf_pool, Some(rte_pktmbuf_init), ptr::null_mut());
    // --- MANUAL MEMORY ALLOCATION END ---

    // The rest is standard configuration...
    let port_conf: rte_eth_conf = std::mem::zeroed();
    let rx_rings: u16 = 1;
    let tx_rings: u16 = 1;
    let mut nb_rxd = RING_SIZE;
    let mut nb_txd = RING_SIZE;

    let mut dev_info: rte_eth_dev_info = std::mem::zeroed();
    let retval = rte_eth_dev_info_get(port, &mut dev_info);
    if retval != 0 {
        return Err(retval);
    }

    let retval = rte_eth_dev_configure(port, rx_rings, tx_rings, &port_conf);
    if retval != 0 {
        return Err(retval);
    }

    let retval = rte_eth_dev_adjust_nb_rx_tx_desc(port, &mut nb_rxd, &mut nb_txd);
    if retval != 0 {
        return Err(retval);
    }

    let retval = rte_eth_rx_queue_setup(
        port,
        0,
        nb_rxd,
        rte_eth_dev_socket_id(port) as u32,
        ptr::null(),
        mbuf_pool,
    );
    if retval < 0 {
        eprintln!("Error setting up RX queue: {}", retval);
        return Err(retval);
    }

    let mut txconf = dev_info.default_txconf;
    txconf.offloads = port_conf.txmode.offloads;
    let retval = rte_eth_tx_queue_setup(
        port,
        0,
        nb_txd,
        rte_eth_dev_socket_id(port) as u32,
        &txconf,
    );
    if retval < 0 {
        return Err(retval);
    }

    let retval = rte_eth_dev_start(port);
    if retval < 0 {
        return Err(retval);
    }

    let mut addr: rte_ether_addr = std::mem::zeroed();
    rte_eth_macaddr_get(port, &mut addr);
    
    println!("Port {} MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        port, addr.addr_bytes[0], addr.addr_bytes[1], addr.addr_bytes[2],
        addr.addr_bytes[3], addr.addr_bytes[4], addr.addr_bytes[5]);

    rte_eth_promiscuous_enable(port);

    Ok(())
}



/// Main packet forwarding loop
unsafe fn wire_ports(in_port: u16, out_port: u16) {
    let mut bufs: [*mut rte_mbuf; MAX_PKT_BURST as usize] = [ptr::null_mut(); MAX_PKT_BURST as usize];
    let mut total_forwarded: u64 = 0;
    let mut total_dropped: u64 = 0;

    println!("Starting packet forwarding:");
    println!("  IN:  Port {}", in_port);
    println!("  OUT: Port {}", out_port);

    loop {
        // Receive burst of packets
        let nb_rx = rte_eth_rx_burst(in_port, 0, bufs.as_mut_ptr(), MAX_PKT_BURST);

        if nb_rx > 0 {
            // Send burst to out_port
            let nb_tx = rte_eth_tx_burst(out_port, 0, bufs.as_mut_ptr(), nb_rx);

            total_forwarded += nb_tx as u64;
            if nb_tx > 0 {
                println!("Total forwarded packets: {}", total_forwarded);
                println!("Total dropped packets: {}", total_dropped);
            }

            // Free any packets that weren't sent
            if nb_tx < nb_rx {
                total_dropped += (nb_rx - nb_tx) as u64;
                for i in nb_tx..nb_rx {
                    rte_pktmbuf_free(bufs[i as usize]);
                }
            }
        }
    }
}

fn main() {
    unsafe {
        // Collect command line arguments as CStrings
        // We need to keep the CStrings alive for the duration of rte_eal_init
        let args: Vec<CString> = std::env::args()
            .map(|arg| CString::new(arg).unwrap())
            .collect();

        // Create a vector of raw pointers
        let mut c_args: Vec<*mut c_char> = args
            .iter()
            .map(|arg| arg.as_ptr() as *mut c_char)
            .collect();

        // Add NULL terminator (some systems expect it)
        c_args.push(ptr::null_mut());

        let argc = (c_args.len() - 1) as c_int;  // Exclude NULL terminator from count
        let argv = c_args.as_mut_ptr();

        // Initialize DPDK EAL
        // Note: rte_eal_init modifies argc and argv to consume EAL arguments
        let ret = rte_eal_init(argc, argv);
        if ret < 0 {
            eprintln!("Error with EAL initialization");
            std::process::exit(1);
        }

        // Calculate how many arguments remain after EAL consumed some
        let remaining_argc = (argc - ret) as usize;

        // Get the remaining arguments (application-specific args after --)
        if remaining_argc != 2 {
            println!("Usage: reflector [EAL options] -- <port_id>");
            println!("Example: sudo ./reflector -l 0 --no-huge --no-pci --vdev 'net_pcap0,rx_pcap=test.pcap,tx_pcap=out.pcap' -- 0");
            std::process::exit(1);
        }

        // Parse port_id from the remaining arguments
        // argv now points to the remaining arguments after EAL processing
        let remaining_argv = std::slice::from_raw_parts(argv.offset(ret as isize), remaining_argc);
        let port_str = std::ffi::CStr::from_ptr(remaining_argv[1]).to_str().unwrap();
        let port_id: u16 = port_str.parse().unwrap_or_else(|_| {
            eprintln!("Invalid port number");
            std::process::exit(1);
        });

        // Initialize the port
        if let Err(e) = port_init(port_id) {
            eprintln!("Cannot init port {}: error {}", port_id, e);
            std::process::exit(1);
        }

        println!("Starting single-port loopback on port {}", port_id);
        println!("Packets received on port {} will be sent back out port {}", port_id, port_id);

        // Run loopback directly in main thread
        wire_ports(port_id, port_id);
    }
}
