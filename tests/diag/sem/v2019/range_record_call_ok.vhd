-- yavs-test: std=2019
-- expect: none
-- LRM 5.2.1 (2019): a function call, a qualified expression or an operator of a range record
-- type is a range; a call with a range record argument stays a call
entity e is generic (n : integer := 3); end entity;
architecture a of e is
  subtype ir is integer'range'record;
  type rt is record a : bit_vector; end record;
  function up(a, b : integer) return ir is begin return (left => a, right => b, direction => ascending); end function;
  function "-"(r : ir) return ir is begin return r; end function;
  function len(r : ir) return integer is begin return 1; end function;
  function ones(r : ir) return bit_vector is variable b : bit_vector(r); begin return b; end function;
  signal x : bit_vector(up(0, n));
  subtype s is integer range up(1, n);
  signal r : rt(a(up(1, 2)));
begin
  process
    variable w : bit_vector(0 to 7);
    variable k : integer;
  begin
    for i in -up(1, 2) loop end loop;
    for i in ir'(left => 1, right => 2, direction => ascending) loop end loop;
    w(up(1, 2)) := "00";
    w((up(1, 2))) := "00";
    k := len(up(1, 2));
    w(0 to 1) := ones(up(1, 2));
    wait;
  end process;
end architecture;
