use glib_build_tools::compile_resources;

fn main() {
    compile_resources(
        &["data"],
        "data/gresource.xml",
        "org.techmore.mseries.Torrential.gresource",
    );
}
