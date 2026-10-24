// swift-tools-version: 6.0
// Fixture for Apple Dev screenshots and tests: the shape `xtool new HelloOmarchy` writes.
import PackageDescription

let package = Package(
    name: "HelloOmarchy",
    platforms: [.iOS(.v17)],
    products: [.library(name: "HelloOmarchy", targets: ["HelloOmarchy"])],
    targets: [.target(name: "HelloOmarchy")]
)
