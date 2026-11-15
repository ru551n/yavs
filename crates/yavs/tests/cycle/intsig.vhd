-- Signals of arrays of integers and enumerations in cycle mode against the normal mode (`W` is
-- replaced by the element count): an array the testbench drives with values that differ per element
-- (read by members, element by element and at a run-time index), an array of arrays through a
-- clocked pipeline (next-state blocks), a memory written and read at run-time indices (from 65
-- elements on, its writes wait for the commit; the testbench reads it only through `mr`), an array of
-- an enumeration, negative values, and a port collapsed onto a slice of an array of integers. `rev`
-- covers the boundary of 8-byte integers and of enumerations in both directions: arrays the
-- testbench drives a few elements of at a time, copied reversed into arrays it reads whole.
package cycle_intsig_pkg is
  type int_arr is array (natural range <>) of integer range -1000 to 1000;
  type wide_arr is array (natural range <>) of integer;
  type lanes_t is array (0 to 2) of int_arr(0 to W - 1);
  type st_t is (s0, s1, s2, s3);
  type st_arr is array (natural range <>) of st_t;
  type big_int is range -2 ** 40 to 2 ** 40;
  type big_arr is array (natural range <>) of big_int;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_intsig_pkg.all;
entity cycle_intsig_sub is
  port (clk : in std_ulogic; a : in int_arr(0 to 1); q : out integer);
end entity;
architecture a of cycle_intsig_sub is
begin
  diff : process (clk)
  begin
    if rising_edge(clk) then
      q <= a(0) - 2 * a(1);
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_intsig_pkg.all;
entity cycle_intsig is
end entity;
architecture a of cycle_intsig is
  signal clk : std_ulogic := '0';
  signal idx : unsigned(7 downto 0) := x"00";
  signal src : int_arr(0 to W - 1) := (others => 0);
  signal both : int_arr(0 to W) := (others => 0);
  signal p : lanes_t := (others => (others => 0));
  signal m : wide_arr(0 to W - 1) := (others => 7);
  signal st : st_arr(0 to W - 1);
  signal sum, mr, q : integer := 0;
  signal bs, bq : big_arr(0 to W - 1) := (others => 0);
  signal es, eq : st_arr(0 to W - 1) := (others => s0);
begin
  pipe : process (clk)
  begin
    if rising_edge(clk) then
      p(0) <= src;
      p(1) <= p(0);
      p(2) <= p(1);
    end if;
  end process;

  comb : process (all)
  begin
    sum <= p(2)(0) + p(2)(W - 1) + src(to_integer(idx) mod W);
  end process;

  wr : process (clk)
  begin
    if rising_edge(clk) then
      m(to_integer(idx) mod W) <= src(0) - p(1)(W - 1);
      mr <= m((to_integer(idx) + 3) mod W);
    end if;
  end process;

  states : process (all)
  begin
    for i in 0 to W - 1 loop
      if src(i) > 300 then
        st(i) <= s1;
      elsif src(i) < -300 then
        st(i) <= s3;
      else
        st(i) <= s2;
      end if;
    end loop;
  end process;

  rev : process (clk)
  begin
    if rising_edge(clk) then
      for i in 0 to W - 1 loop
        bq(i) <= bs(W - 1 - i) - big_int(i);
        eq(i) <= es(W - 1 - i);
      end loop;
    end if;
  end process;

  -- Two elements of `both`, collapsed into the port.
  sub : entity work.cycle_intsig_sub port map (clk => clk, a => both(W - 1 to W), q => q);

  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
    variable bsum : big_int;
    variable esum : integer;
  begin
    wait for 1 ns;
    for c in 1 to 20 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      idx <= r(7 downto 0);
      for i in 0 to W - 1 loop
        if (c + i) mod 3 /= 0 then
          src(i) <= (c * 37 + i * 113 + to_integer(r(5 downto 0))) mod 2001 - 1000;
        end if;
      end loop;
      bs((c * 7) mod W) <= big_int(c) * 2 ** 33 - 5;
      bs((c * 3) mod W) <= -big_int(c) * 2 ** 35 + big_int(c);
      es((c * 5) mod W) <= st_t'val(c mod 4);
      both(W) <= c * 3 - 30;
      both(W - 1) <= 500 - c * 11;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      bsum := 0;
      esum := 0;
      for i in 0 to W - 1 loop
        bsum := bsum + (bq(i) mod 1000003) * big_int(i + 1);
        esum := esum + st_t'pos(eq(i)) * (i + 1);
      end loop;
      report "c=" & integer'image(c) & " sum=" & integer'image(sum) & " mr=" & integer'image(mr)
        & " q=" & integer'image(q) & " p2=" & integer'image(p(2)(0)) & "," & integer'image(p(2)(W - 1))
        & " st=" & st_t'image(st(0)) & st_t'image(st(W - 1))
        & " bq=" & big_int'image(bq(0)) & "," & big_int'image(bq(W - 1)) & " bsum=" & big_int'image(bsum)
        & " eq=" & st_t'image(eq(0)) & " esum=" & integer'image(esum);
    end loop;
    wait;
  end process;
end architecture;
