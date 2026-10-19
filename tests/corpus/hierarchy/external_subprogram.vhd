-- difftest: top=external_subprogram std=2008 ignore=order
-- ignore=order: two processes report in the same cycle at 6 ns (h2 and main); the LRM leaves
-- their order unspecified.
-- External names in subprograms (LRM 8.7, #255): procedures and functions of a package body
-- force, release, read and wait on signals of the design and read its constants through
-- absolute paths, bound once the hierarchy is elaborated and shared by every caller (two
-- harness instances call them); a function of the architecture reads through a relative
-- path; package paths name objects of other packages, also in a package declaration.
package types_pkg is
  type int_vec is array (natural range <>) of integer;
  type pair is record
    a : integer;
    b : bit;
  end record;
  constant base : integer := 3;
  signal shared_flag : bit := '0';
end package;

use work.types_pkg.all;
package ops_pkg is
  -- A package path in a package declaration: another package's constant.
  constant scaled : integer := <<constant @work.types_pkg.base : integer>> * 7;
  procedure poke(v : bit);
  procedure unpoke;
  impure function peek return bit_vector;
  impure function width return integer;
  impure function table return int_vec;
  impure function cfg return pair;
  procedure wait_count(n : integer);
end package;

package body ops_pkg is
  procedure poke(v : bit) is
  begin
    <<signal .external_subprogram.dut.flag : bit>> <= force v;
  end procedure;
  procedure unpoke is
  begin
    <<signal .external_subprogram.dut.flag : bit>> <= release;
  end procedure;
  impure function peek return bit_vector is
    alias b is <<signal .external_subprogram.dut.data : bit_vector(3 downto 0)>>;
  begin
    return b;
  end function;
  impure function width return integer is
  begin
    return <<constant .external_subprogram.dut.w : integer>>;
  end function;
  impure function table return int_vec is
  begin
    return <<constant .external_subprogram.dut.tbl : int_vec>>;
  end function;
  impure function cfg return pair is
  begin
    return <<constant .external_subprogram.dut.p : pair>>;
  end function;
  procedure wait_count(n : integer) is
  begin
    wait until <<signal .external_subprogram.dut.count : integer>> >= n;
  end procedure;
end package body;

use work.types_pkg.all;
entity dut is
  generic (n : integer := 2);
end;
architecture a of dut is
  signal data : bit_vector(3 downto 0) := "0001";
  signal flag : bit := '0';
  signal count : integer := 0;
  constant w : integer := n * 4;
  constant tbl : int_vec(1 to 3) := (n, n + 1, n + 2);
  constant p : pair := (a => n * 100, b => '1');
begin
  process (flag)
  begin
    report "dut flag=" & bit'image(flag);
    data <= data(2 downto 0) & flag;
  end process;
  process
  begin
    for i in 1 to 4 loop
      wait for 2 ns;
      count <= i;
    end loop;
    wait;
  end process;
end;

use work.ops_pkg.all;
entity harness is
  generic (id : integer);
end;
architecture h of harness is
begin
  process
  begin
    wait_count(id + 1);
    report "harness" & integer'image(id) & " saw count " & integer'image(id + 1)
      & " width=" & integer'image(width);
    wait;
  end process;
end;

use work.types_pkg.all;
use work.ops_pkg.all;
entity external_subprogram is end;
architecture tb of external_subprogram is
  impure function dut_count return integer is
  begin
    return <<signal dut.count : integer>>;
  end function;
begin
  dut : entity work.dut generic map (n => 5);
  h1 : entity work.harness generic map (id => 1);
  h2 : entity work.harness generic map (id => 2);
  main : process
    variable t : int_vec(1 to 3);
    variable c : pair;
  begin
    report "scaled=" & integer'image(scaled) & " width=" & integer'image(width);
    t := table;
    c := cfg;
    report "table=" & integer'image(t(1)) & "," & integer'image(t(2)) & ","
      & integer'image(t(3)) & " cfg=" & integer'image(c.a) & "," & bit'image(c.b);
    assert scaled = 21 and width = 20 and t(3) = 7 and c.a = 500 severity failure;
    poke('1');
    wait for 1 ns;
    report "peek=" & to_string(peek);
    assert peek = "0101" severity failure;
    unpoke;
    wait for 1 ns;
    report "peek=" & to_string(peek);
    wait_count(3);
    report "dut_count=" & integer'image(dut_count);
    assert dut_count = 3 severity failure;
    <<signal @work.types_pkg.shared_flag : bit>> <= '1';
    wait for 0 ns;
    report "flag=" & bit'image(shared_flag);
    wait;
  end process;
end;
