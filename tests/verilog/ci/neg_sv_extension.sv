// A .sv file is refused by its extension, whatever it contains.
module sv_ext (input a, output y);
    assign y = a;
endmodule
