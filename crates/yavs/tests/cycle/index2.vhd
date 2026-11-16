-- Elements of elements, both chosen at run time, assigned (`a(i)(j) <=`), against the normal mode
-- (`W` replaced by a width): a signal of vectors, one of `to` vectors, a slice of each element, a
-- field of an array of records, a variable; generic `BAD` (1 or 2) makes the inner or the outer
-- index leave its range.
library ieee;
use ieee.std_logic_1164.all;
entity cycle_index2 is
  generic (BAD : natural := 0);
end entity;
architecture t of cycle_index2 is
  type vecs_t is array (0 to 5) of std_ulogic_vector(W - 1 downto 0);
  type tos_t is array (2 downto 0) of std_ulogic_vector(0 to W - 1);
  type wide_t is array (0 to 3) of std_ulogic_vector(W + 7 downto 0);
  type rec_t is record
    tag : std_ulogic_vector(2 downto 0);
    v : std_ulogic_vector(W - 1 downto 0);
  end record;
  type recs_t is array (0 to 2) of rec_t;
  signal clk, b : std_ulogic := '0';
  signal i : natural range 0 to 7 := 0;
  signal j : natural := 0;
  signal m : vecs_t := (others => (others => '0'));
  signal tt : tos_t := (others => (others => '0'));
  signal ws : wide_t := (others => (others => '0'));
  signal rs : recs_t := (others => ((others => '0'), (others => '0')));
  signal vo : std_ulogic_vector(W - 1 downto 0);
begin
  wr : process (clk)
    variable v : vecs_t := (others => (others => '0'));
  begin
    if rising_edge(clk) then
      m(i mod 6)(j) <= b;
      tt(i mod 3)(j) <= not b;
      ws(i mod 4)(W + 3 downto 4)(j + 4) <= b;
      rs(i mod 3).v(j) <= b;
      rs(i mod 3).tag(j mod 3) <= b;
      if BAD = 1 then
        m(i mod 6)(j + W - 2) <= b;
      elsif BAD = 2 then
        m(i)(j) <= b;
      end if;
      v((i + 1) mod 6)(j) := not b;
      vo <= v(i mod 6);
    end if;
  end process;
  main : process
    variable r : std_ulogic_vector(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 16 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      b <= r(0);
      i <= (k * 5) mod 8;
      j <= (k * 3) mod W;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " m=" & to_hstring(m(0)) & to_hstring(m(3)) & to_hstring(m(5))
        & " tt=" & to_string(tt(0)) & to_string(tt(2)) & " ws=" & to_hstring(ws(1)) & to_hstring(ws(3))
        & " rs=" & to_hstring(rs(0).v) & to_hstring(rs(2).v) & to_string(rs(1).tag) & " vo=" & to_hstring(vo);
    end loop;
    wait;
  end process;
end architecture;
