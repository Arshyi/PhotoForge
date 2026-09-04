//! Authoring minimal ONNX graphs, for tests.
//!
//! ONNX is protobuf and nothing else, so a small encoder is enough to write a
//! valid model. This exists so the inference path can be tested end to end
//! without committing a model binary, downloading anything, or requiring
//! Python — the graph is built in memory by the test that uses it.
//!
//! These are toy graphs. They prove the plumbing — parsing, tiling, tensor
//! layout, colour conversion, seam blending — and prove nothing about the
//! quality of any real published model.

/// Minimal protobuf writing.
mod pb {
    pub fn varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn tag(out: &mut Vec<u8>, field: u32, wire: u32) {
        varint(out, u64::from((field << 3) | wire));
    }

    pub fn int_field(out: &mut Vec<u8>, field: u32, value: i64) {
        tag(out, field, 0);
        varint(out, value as u64);
    }

    pub fn bytes_field(out: &mut Vec<u8>, field: u32, value: &[u8]) {
        tag(out, field, 2);
        varint(out, value.len() as u64);
        out.extend_from_slice(value);
    }

    pub fn string_field(out: &mut Vec<u8>, field: u32, value: &str) {
        bytes_field(out, field, value.as_bytes());
    }
}

/// `TensorProto.DataType.FLOAT`.
const FLOAT: i64 = 1;

fn value_info(name: &str, shape: &[i64]) -> Vec<u8> {
    let mut dims = Vec::new();
    for extent in shape {
        let mut dim = Vec::new();
        // A negative extent means a symbolic dimension, which is how a model
        // says it accepts any tile size.
        if *extent < 0 {
            pb::string_field(&mut dim, 2, "n");
        } else {
            pb::int_field(&mut dim, 1, *extent);
        }
        pb::bytes_field(&mut dims, 1, &dim);
    }
    let mut tensor = Vec::new();
    pb::int_field(&mut tensor, 1, FLOAT);
    pb::bytes_field(&mut tensor, 2, &dims);
    let mut type_proto = Vec::new();
    pb::bytes_field(&mut type_proto, 1, &tensor);
    let mut info = Vec::new();
    pb::string_field(&mut info, 1, name);
    pb::bytes_field(&mut info, 2, &type_proto);
    info
}

fn initializer(name: &str, shape: &[i64], values: &[f32]) -> Vec<u8> {
    let mut tensor = Vec::new();
    for extent in shape {
        pb::int_field(&mut tensor, 1, *extent);
    }
    pb::int_field(&mut tensor, 2, FLOAT);
    pb::string_field(&mut tensor, 8, name);
    let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    pb::bytes_field(&mut tensor, 9, &raw);
    tensor
}

fn node(op_type: &str, name: &str, inputs: &[&str], outputs: &[&str]) -> Vec<u8> {
    let mut node = Vec::new();
    for input in inputs {
        pb::string_field(&mut node, 1, input);
    }
    for output in outputs {
        pb::string_field(&mut node, 2, output);
    }
    pb::string_field(&mut node, 3, name);
    pb::string_field(&mut node, 4, op_type);
    node
}

fn wrap(graph: Vec<u8>) -> Vec<u8> {
    let mut opset = Vec::new();
    pb::string_field(&mut opset, 1, "");
    pb::int_field(&mut opset, 2, 13);
    let mut model = Vec::new();
    pb::int_field(&mut model, 1, 7); // ir_version
    pb::string_field(&mut model, 2, "photoforge-test");
    pb::bytes_field(&mut model, 7, &graph);
    pb::bytes_field(&mut model, 8, &opset);
    model
}

/// A model that computes `y = x * scale + offset`, elementwise.
///
/// Chosen because a correct run is distinguishable from every common failure:
/// an identity, a zero tensor, a transposed layout and a pass-through all give
/// visibly different answers.
pub fn scale_and_offset(width: i64, height: i64, scale: f32, offset: f32) -> Vec<u8> {
    let mut graph = Vec::new();
    pb::bytes_field(&mut graph, 1, &node("Mul", "scale", &["x", "s"], &["t"]));
    pb::bytes_field(&mut graph, 1, &node("Add", "offset", &["t", "o"], &["y"]));
    pb::string_field(&mut graph, 2, "photoforge-scale-offset");
    pb::bytes_field(&mut graph, 5, &initializer("s", &[1], &[scale]));
    pb::bytes_field(&mut graph, 5, &initializer("o", &[1], &[offset]));
    pb::bytes_field(&mut graph, 11, &value_info("x", &[1, 3, height, width]));
    pb::bytes_field(&mut graph, 12, &value_info("y", &[1, 3, height, width]));
    wrap(graph)
}

/// A model whose declared output channel count disagrees with what it produces.
///
/// Used to check the runtime refuses a model that does not match its
/// descriptor, rather than reading past the end of a tensor.
pub fn single_channel_output(width: i64, height: i64) -> Vec<u8> {
    let mut graph = Vec::new();
    pb::bytes_field(
        &mut graph,
        1,
        &node("ReduceMean", "collapse", &["x"], &["y"]),
    );
    pb::string_field(&mut graph, 2, "photoforge-collapse");
    pb::bytes_field(&mut graph, 11, &value_info("x", &[1, 3, height, width]));
    pb::bytes_field(&mut graph, 12, &value_info("y", &[1, 1, height, width]));
    wrap(graph)
}

/// Bytes that begin like an ONNX file and are not one.
pub fn truncated() -> Vec<u8> {
    let mut bytes = scale_and_offset(8, 8, 0.5, 0.0);
    bytes.truncate(bytes.len() / 2);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoder has to produce something PhotoForge's own import check
    /// accepts, or the fixtures would be testing a path users never take.
    #[test]
    fn authored_models_look_like_onnx_to_the_import_check() {
        let bytes = scale_and_offset(16, 16, 0.5, 0.25);
        assert_eq!(
            bytes[0],
            super::super::model::ModelFormat::Onnx.leading_byte()
        );
        assert!(bytes.len() > 64, "the authored model is implausibly small");
        assert!(
            bytes.len() < 4096,
            "the authored model is larger than expected"
        );
    }

    #[test]
    fn varint_encodes_multi_byte_values() {
        let mut out = Vec::new();
        pb::varint(&mut out, 0);
        assert_eq!(out, vec![0]);
        out.clear();
        pb::varint(&mut out, 300);
        assert_eq!(out, vec![0xac, 0x02]);
    }
}
