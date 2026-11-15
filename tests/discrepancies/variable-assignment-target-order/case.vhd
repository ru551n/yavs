-- Order of a variable assignment's target name and value (LRM 10.6.2.1 says both are evaluated
-- first, in no order): an impure function in the index and one in the value report when they run,
-- numbered, for a scalar element and for an element that is an array; and a whole object through
-- an access value whose value reassigns the access variable.
entity top is end entity;
architecture t of top is
  type iv is array (natural range <>) of integer;
  type bv is array (natural range <>) of bit_vector(1 downto 0);
  type pair is array (0 to 1) of integer;
  type pair_ptr is access pair;
begin
  process is
    variable v : iv(0 to 3);
    variable w : bv(0 to 3);
    variable seq : natural := 0;
    variable p : pair_ptr := new pair'(0, 0);
    variable q : pair_ptr;
    impure function t(k : natural; tag : string) return natural is
    begin
      seq := seq + 1;
      report "RES " & integer'image(seq) & " " & tag & " target";
      return k;
    end function;
    impure function x(tag : string) return integer is
    begin
      seq := seq + 1;
      report "RES " & integer'image(seq) & " " & tag & " value";
      return 5;
    end function;
    impure function y(tag : string) return bit_vector is
    begin
      seq := seq + 1;
      report "RES " & integer'image(seq) & " " & tag & " value";
      return "10";
    end function;
    impure function z return pair is
    begin
      q := p;
      p := new pair'(7, 7);
      return (3, 3);
    end function;
  begin
    v(t(1, "scalar")) := x("scalar");
    w(t(1, "array")) := y("array");
    p.all := z;
    report "RES access: old object " & integer'image(q.all(0)) & ", new object " & integer'image(p.all(0));
    wait;
  end process;
end architecture;
