-- difftest: top=read_bit_vector_white_space std=2008
-- READ of a bit_vector accepts white space before each bit (as a READ of each BIT would;
-- LRM 16.4 is silent, L131). The first reference simulator does so (this .expected is its
-- output), the second does not (tests/discrepancies/textio-read-bitvector-white-space).
use std.textio.all;
entity read_bit_vector_white_space is end;
architecture t of read_bit_vector_white_space is
begin
  process
    variable l : line;
    variable y : bit_vector(7 downto 0) := (others => '0');
    variable z : bit_vector(3 downto 0) := (others => '0');
    variable good : boolean;
  begin
    l := new string'("1 1 0 0 0 0 1 1");
    read(l, y, good);
    assert good and y = "11000011" report "spaces" severity failure;
    assert l'length = 0 report "spaces rest" severity failure;
    l := new string'("  1100 1");
    read(l, z, good);
    assert good and z = "1100" report "leading" severity failure;
    assert l.all = " 1" report "leading rest" severity failure;
    l := new string'("1" & HT & "10 1");
    read(l, z, good);
    assert good and z = "1101" report "tab" severity failure;
    l := new string'("1 1x0");
    read(l, z, good);
    assert not good report "bad" severity failure;
    report "done";
    wait;
  end process;
end;
