//! The VHDL view of a module (`yavs_verilog::interface`): parameters at their defaults, ports with
//! the widths those give, and the reasons a module has no view.

use yavs_verilog::{Interface, Options, ast, interface, parse_str};

fn view(src: &str) -> Interface {
    let decls = parse_str(src, &Options::default()).expect("parses");
    let m = decls
        .iter()
        .find_map(|d| match d {
            ast::Decl::Module(m) => Some(m),
            _ => None,
        })
        .expect("a module");
    interface(m, Options::default().default_timescale)
}

#[test]
fn parameters_and_ports_at_their_defaults() {
    let i = view(
        "module m #(parameter W = 8, parameter [3:0] V = 4'b10x1, parameter S = \"TRUE\",
                    parameter real R = 1.5, parameter integer N = -3, localparam L = 2)
                   (input clk, input [W-1:0] a, output reg [0:L] q, inout [1:0] io);
           reg [3:0] mem [0:3];
         endmodule",
    );
    assert_eq!(
        i.encode(),
        "param W int 8\nparam V vec 4 10x1\nparam S str 54525545\nparam R real 1.5\nparam N int -3\n\
         port clk in scalar\nport a in 7 0\nport q out 0 2\nport io inout 1 0\n"
    );
    assert_eq!(Interface::decode(&i.encode()), Some(i));
}

#[test]
fn non_ansi_ports_take_their_range_from_the_body() {
    let i =
        view("module m (a, q); parameter W = 4; input [W-1:0] a; output q; reg [1:0] q; endmodule");
    assert_eq!(i.encode(), "param W int 4\nport a in 3 0\nport q out 1 0\n");
}

#[test]
fn what_the_view_cannot_say_is_an_error() {
    let e = view("module m (output real r); endmodule")
        .error
        .expect("an error");
    assert!(e.contains("real"), "{e}");
}
