// Run YCSB benchmark through the RepCXL client library. It imports
// a YCSB workload and executes it (rather than issuing reqs from YCSB Java bin).

use rep_cxl::utils::ycsb::load_ycsb_workload;
use rep_cxl::utils::arg_parser::ArgParser;
use clap::{Arg, value_parser};
use log::info;
fn main() {

    simple_logger::init().unwrap();

    let mut ap = ArgParser::new( "ycsb_client", "YCSB Client for RepCXL" ); 
    
    // Add benchmark-specific arguments
    ap.add_args(&[
        Arg::new("load_trace")
            .help("Path to the YCSB load trace file")
            .required(true)
            .index(1)
            .value_parser(value_parser!(String)),
        Arg::new("run_trace")
            .help("Path to the YCSB run trace file")
            .required(true)
            .index(2)
            .value_parser(value_parser!(String)),
    ]);
    
    let extra_args = ap.parse();
    
    let load_trace = extra_args.get_one::<String>("load_trace").unwrap();
    let run_trace = extra_args.get_one::<String>("run_trace").unwrap();

    // println!("\nParsed arguments: {:#?}", ap);

    let workload = load_ycsb_workload(load_trace, run_trace);
    workload.summary();

    info!("\nFirst 5 load operations:");
    for (i, op) in workload.load_ops.iter().enumerate().take(5) {
        let val_preview: String = op.fields.first()
            .map(|(name, val)| format!(" {}=[{}B]", name, val.len()))
            .unwrap_or_default();
        info!("  [{}] {:?} {}{}", i, op.op_type, op.key, val_preview);
    }

    info!("\nFirst 10 run operations:");
    for (i, op) in workload.run_ops.iter().enumerate().take(10) {
        let val_preview: String = op.fields.first()
            .map(|(name, val)| format!(" {}=[{}B]", name, val.len()))
            .unwrap_or_default();
        info!("  [{}] {:?} {}{}", i, op.op_type, op.key, val_preview);
    }
    if workload.run_ops.len() > 10 {
        info!("  ... ({} more)", workload.run_ops.len() - 10);
    }

    

}