-- difftest: top=aggregate_target_signal_param std=2008
-- Aggregate targets over signal parameters: scalars, slices, elements and record fields.
entity aggregate_target_signal_param is end;
architecture test of aggregate_target_signal_param is
  type rec_t is record a : bit_vector(0 to 5); b : bit_vector(0 to 1); c : bit; end record;
  type ar is array (0 to 1) of bit_vector(0 to 1);
  procedure des (signal bv : in bit_vector; signal rec : out rec_t) is
  begin
    (rec.a, rec.b, rec.c) <= bv;
  end procedure;
  procedure sc (signal x, y : out bit; signal v : out bit_vector; v2 : bit_vector) is
  begin
    (x, y, v) <= v2 after 1 ns, not v2 after 3 ns;
  end procedure;
  procedure el (signal v : out bit_vector) is
  begin
    (v(0), v(1)) <= bit_vector'("10");
  end procedure;
  signal r  : rec_t;
  signal bv : bit_vector(0 to 8) := "101100111";
  signal x, y : bit;
  signal v : bit_vector(0 to 1);
  signal w : bit_vector(0 to 1);
begin
  process begin 
    des(bv, r); sc(x, y, v, "1101"); el(w);
    wait for 2 ns; report to_string(r.a) & " " & to_string(r.b) & " " & bit'image(r.c);
    report bit'image(x) & bit'image(y) & to_string(v) & to_string(w);
    wait for 2 ns;
    report bit'image(x) & bit'image(y) & to_string(v);
    wait; end process;
end architecture;
