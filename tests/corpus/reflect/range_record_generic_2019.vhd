-- difftest: top=range_record_generic_2019 std=2019 ref=hand ignore=order
-- ref=hand: LCS-2016-099: a generic of a range record type constrains a port and drives a
-- for-generate, in two instances with opposite directions; a process (all) reads a signal of
-- a range record type through a loop and a slice. No reference simulator runs it (L112); the
-- expected output was checked by hand. A conditional constant (2019) sizes a signal.
-- ignore=order: the generate processes of the two instances report in the same cycle.
package range_record_generic_pkg is
  subtype ir is integer'range'record;
end package;

use work.range_record_generic_pkg.all;
entity range_record_generic_child is
  generic (g : ir);
  port (p : in bit_vector(g));
end entity;
architecture t of range_record_generic_child is
begin
  gen : for i in g generate
    process
    begin
      wait for 1 ns;
      report "g " & integer'image(i) & " " & bit'image(p(i));
      wait;
    end process;
  end generate;
  process
  begin
    report "p " & integer'image(p'left) & " " & integer'image(p'right) & " " & boolean'image(p'ascending);
    wait;
  end process;
end architecture;

use work.range_record_generic_pkg.all;
entity range_record_generic_2019 is
  generic (wide : boolean := true);
end entity;
architecture t of range_record_generic_2019 is
  constant width : integer := 4 when wide else 2;
  signal c : bit_vector(width - 1 downto 0) := "1100";
  signal sr : ir := (left => 0, right => 1, direction => ascending);
  signal w : bit_vector(0 to 3) := "0110";
  signal n : integer := 0;
begin
  up : entity work.range_record_generic_child
    generic map (g => (left => 1, right => 2, direction => ascending))
    port map (p => "10");
  down : entity work.range_record_generic_child
    generic map (g => (left => 2, right => 1, direction => descending))
    port map (p => "10");
  -- Sensitive to `sr` through its loop range and its slice range (LRM 11.3).
  process (all)
    variable k : integer;
  begin
    k := 0;
    for i in sr loop
      k := k * 10 + i;
    end loop;
    n <= k;
    report "all " & integer'image(k) & " " & to_string(w(sr)) & " " & integer'image(c'length);
  end process;
  process
  begin
    wait for 2 ns;
    sr <= (left => 1, right => 3, direction => ascending);
    wait;
  end process;
end architecture;
