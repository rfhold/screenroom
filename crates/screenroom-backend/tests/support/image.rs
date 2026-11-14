pub fn png(color: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(2, 2, image::Rgb(color));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}
