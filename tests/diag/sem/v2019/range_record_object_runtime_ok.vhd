-- yavs-test: std=2019
-- expect: none
-- LRM 5.3.2.2 (2019): an object of a range record type known only at run time is a range
entity e is generic (n : integer := 3); end entity;
architecture a of e is
  subtype ir is integer'range'record;
  function f(r : ir) return integer is begin return r.left; end function;
  function f(r : ir; k : integer) return integer is begin return k; end function;
  signal s : bit_vector(n downto 0);
  constant r : s'range'record := s'range'value;
  signal x : bit_vector(r);
  signal sr : ir;
begin
  process
    variable rv : ir := r;
    variable v : bit_vector(rv);
    variable w : bit_vector(7 downto 0);
    variable k : integer;
  begin
    for i in natural loop exit; end loop;
    for i in sr loop k := f(rv) + f(rv, i); end loop;
    w(rv) := v;
    wait;
  end process;
end architecture;
