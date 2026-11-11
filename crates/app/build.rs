fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon.ico");
        if let Err(e) = res.compile() {
            // Not fatal: the app still works without an embedded icon.
            println!("cargo:warning=failed to embed Windows resources: {e}");
        }
    }
}
