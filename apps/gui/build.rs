//! Puts the logo and the version information in the Windows executable. If the resource
//! compiler is missing, the program is built without them.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/easylidar.ico")
            .set("ProductName", "EasyLiDAR")
            .set(
                "FileDescription",
                "EasyLiDAR: procesamiento y edición de nubes LiDAR para forestales",
            )
            .set("CompanyName", "Julio M. Sevilla")
            .set("LegalCopyright", "© 2026 Julio M. Sevilla");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no se pudo incrustar el icono: {e}");
        }
    }
}
