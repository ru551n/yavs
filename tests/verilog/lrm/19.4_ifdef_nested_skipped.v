// LRM 19.4: a conditional nested in a skipped branch is skipped with its `else and `endif
// units: c
// expect: none
`timescale 1ns/1ps
`ifdef X
`ifdef Y
module a;
endmodule
`else
module b;
endmodule
`endif
`else
module c;
endmodule
`endif
