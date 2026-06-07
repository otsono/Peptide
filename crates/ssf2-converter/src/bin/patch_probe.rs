//! patch_probe — apply the full install_patched injection set at a FIXED port, for
//! byte-diffing two abc_inject versions. Usage: patch_probe <in.swf> <out.swf>
use ssf2_converter::abc_inject;
fn main() {
    let inp = std::env::args().nth(1).unwrap();
    let out = std::env::args().nth(2).unwrap();
    abc_inject::patch_file_with(inp.as_ref(), out.as_ref(), |abc| {
        abc_inject::inject_socket_bridge(abc, "com.mcleodgaming.ssf2.Main", "127.0.0.1", 19000)?;
        abc_inject::inject_input_applicator(abc, "com.mcleodgaming.ssf2.Main")?;
        abc_inject::inject_quickboot(abc, "mario", "battlefield")?;
        abc_inject::inject_jump_probe(abc, "com.mcleodgaming.ssf2.Main", "/tmp/traj.csv", 0)?;
        Ok(())
    }).unwrap();
    println!("patched -> {out}");
}
