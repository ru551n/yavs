// A synthetic protected envelope (not a real key or payload): the body is never lexed.
`timescale 1ns / 1ps
module before_envelope (input a, output y);
    assign y = a;
endmodule
`pragma protect begin_protected
`pragma protect version = 1
`pragma protect encrypt_agent = "EXAMPLE"
`pragma protect data_method = "aes128-cbc"
`pragma protect data_block
dGhpcyBpcyBub3QgdmVyaWxvZyAiIC8qIGVuZG1vZHVsZSBgZW5kaWYgJCUmIw==
"unterminated string /* open comment `ifdef
`pragma protect end_protected
