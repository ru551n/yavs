-- difftest: top=constant_bounds
-- A constant of a constrained array subtype has the bounds of its subtype, not those of its
-- initial value (LRM 6.4.2.2, 5.3.2.2), however it is read: 'left/'right/'range, indexing,
-- slicing and as the actual of an unconstrained parameter (package, architecture, process).
package p is
  constant pc : string(5 to 7) := "abc";
  constant pd : bit_vector(7 downto 4) := "1010";
  constant pi : integer_vector(3 to 5) := (1, 2, 3);
  function id(s : string) return string;
end package;
package body p is
  function id(s : string) return string is begin return s; end function;
end package body;
use work.p.all;
entity constant_bounds is
  generic (n : natural := 3);
end entity;
architecture a of constant_bounds is
  function lft(constant s : string) return integer is begin return s'left; end function;
  function rgt(constant s : string) return integer is begin return s'right; end function;
  constant ac : string(1 to 3) := "xyz";
  constant ad : string(2 to n + 1) := "xyz";
  constant ae : string(8 to 10) := id("xyz");
  constant af : string(9 downto 7) := "xyz";
  constant gb : string(5 to 7) := "xyz";
  constant gl : string(2 to n + 1) := "xyz";
  signal s : bit_vector(1 to gl'right - 2);
begin
  process
    constant lc : string(20 to 22) := "xyz";
    constant ld : string(n + 10 to n + 12) := "xyz";
    variable cnt : natural := 0;
  begin
    assert pc'left = 5 and pc'right = 7 and pc(5) = 'a' and pc(7) = 'c' severity failure;
    assert lft(pc) = 5 and rgt(pc) = 7 report "pc" severity failure;
    assert pd'left = 7 and pd(7) = '1' and pd(4) = '0' severity failure;
    assert pi'left = 3 and pi(3) = 1 and pi(5) = 3 severity failure;
    assert lft(ac) = 1 and lft(ad) = 2 and lft(ae) = 8 and lft(af) = 9 report "arch" severity failure;
    assert rgt(ad) = 4 and rgt(ae) = 10 and rgt(af) = 7 report "arch r" severity failure;
    assert lft(lc) = 20 and lft(ld) = 13 report "proc" severity failure;
    assert lft(pc(6 to 7)) = 6 and lft(ae(9 to 10)) = 9 and lft(lc(21 to 22)) = 21 report "slice" severity failure;
    assert ae(8) = 'x' and ae(10) = 'z' and ad(2) = 'x' and af(9) = 'x' and af(7) = 'z' report "index" severity failure;
    for i in pc'range loop cnt := cnt + i; end loop;
    assert cnt = 18 report "range" severity failure;
    for i in af'range loop cnt := cnt + i; end loop;
    assert cnt = 18 + 24 report "range af" severity failure;
    for i in lc'range loop cnt := cnt + i; end loop;
    assert cnt = 42 + 63 report "range lc" severity failure;
    assert lc'length = 3 and lc'low = 20 and lc'high = 22 and af'ascending = false severity failure;
    report lc & ad & ae & af & pc;
    assert gl'left = 2 and gl(2) = 'x' and lft(gl) = 2 and lft(gb) = 5 report "gen" severity failure;
    report "done";
    assert s'length = 2 severity failure;
    wait;
  end process;

  g : if gl(2) = 'x' and gl'left = 2 and gb(5) = 'x' generate
    process begin
      assert lft(gl) = 2 and lft(gb) = 5 report "elab" severity failure;
      wait for 1 ns;
      report "generate ok";
      wait;
    end process;
  end generate;
end architecture;
