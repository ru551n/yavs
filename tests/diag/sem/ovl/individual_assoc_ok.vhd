-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.1
-- Sub-element (individual) association of subprogram formals: an element and a slice of
-- an unconstrained `out` signal formal, record elements of a variable formal, and an
-- overload that the element types decide.
entity e is end entity;
architecture a of e is
  type rec is record
    i : integer;
    b : bit;
  end record;
  signal tl, tv : bit := '0';
  signal s2 : bit_vector(1 downto 0);
  procedure pr(signal x : out bit_vector) is begin x(x'left) <= '1'; end procedure;
  procedure q(variable r : inout rec) is begin r.i := r.i + 1; end procedure;
  procedure ov(constant v : in bit_vector) is begin end procedure;
  procedure ov(constant v : in integer_vector) is begin end procedure;
  function len(v : bit_vector) return natural is begin return v'length; end function;
begin
  process
    variable vi : integer := 0;
    variable vb : bit := '0';
    variable n : natural;
  begin
    pr(x(0) => tl);
    pr(x(1) => tv, x(0) => tl);
    pr(x(3 downto 2) => s2);
    q(r.i => vi, r.b => vb);
    ov(v(0) => 5);
    ov(v(0) => '1', v(1) => '0');
    n := len(v(0) => '1', v(1) => vb);
    wait;
  end process;
end architecture;
