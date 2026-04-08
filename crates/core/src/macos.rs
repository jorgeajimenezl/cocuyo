/// Copy BGRA pixel data, stripping row padding if present.
pub fn strip_stride_padding(
    src: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
) -> Vec<u8> {
    let stride = width * 4;
    if bytes_per_row == stride {
        let total = stride * height;
        return src[..total.min(src.len())].to_vec();
    }
    let mut bgra = vec![0u8; stride * height];
    for row in 0..height {
        let src_start = row * bytes_per_row;
        if src_start >= src.len() {
            break;
        }
        let available = (src.len() - src_start).min(stride);
        bgra[row * stride..row * stride + available]
            .copy_from_slice(&src[src_start..src_start + available]);
    }
    bgra
}
