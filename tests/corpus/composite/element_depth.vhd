-- difftest: top=element_depth std=2008
-- 'element'element and 'element'element'element attributes of objects whose element
-- bounds are open at every level (#256): the bounds come from the object, for ports,
-- subprogram parameters, signals, variables, aliases and constants.
package element_depth_pkg is
  type r1_t is array (natural range <>) of bit_vector;
  type r2_t is array (natural range <>) of r1_t;
  type r3_t is array (natural range <>) of r2_t;
end;

use work.element_depth_pkg.all;

entity inner is
  port (p2 : in r2_t; p3 : in r3_t);
end;

architecture a of inner is
  alias a2 : r2_t is p2;
begin
  process
  begin
    report "port2 " & integer'image(p2'element'element'left) & " "
      & integer'image(p2'element'element'right) & " " & integer'image(p2'element'element'low)
      & " " & integer'image(p2'element'element'high) & " "
      & integer'image(p2'element'element'length) & " "
      & boolean'image(p2'element'element'ascending);
    report "port3 " & integer'image(p3'element'element'element'left) & " "
      & integer'image(p3'element'element'element'right) & " "
      & integer'image(p3'element'element'element'length) & " "
      & integer'image(p3'element'element'left) & " " & integer'image(p3'element'left) & " "
      & boolean'image(p3'element'element'element'ascending);
    report "alias " & integer'image(a2'element'element'left) & " "
      & integer'image(a2'element'element'length);
    wait;
  end process;
end;

use work.element_depth_pkg.all;

entity element_depth is end;

architecture t of element_depth is
  signal s2 : r2_t(0 to 1)(0 to 2)(4 downto 1);
  signal s3 : r3_t(1 to 1)(2 to 3)(5 to 7)(6 downto 3);
  alias sa : r2_t is s2;
  constant k2 : r2_t(0 to 0)(0 to 1)(3 to 5) := (others => (others => "000"));
  constant k3 : r3_t(0 to 0)(0 to 0)(0 to 0)(9 downto 7) := (others => (others => (others => "000")));

  function f2(m : r2_t) return string is
  begin
    return integer'image(m'element'element'left) & " " & integer'image(m'element'element'right)
      & " " & integer'image(m'element'element'low) & " " & integer'image(m'element'element'high)
      & " " & integer'image(m'element'element'length) & " "
      & boolean'image(m'element'element'ascending) & " " & integer'image(m'element'left);
  end;

  function f3(m : r3_t) return string is
    variable n : natural := 0;
  begin
    for i in m'element'element'element'range loop n := n + 1; end loop;
    return integer'image(m'element'element'element'left) & " "
      & integer'image(m'element'element'element'right) & " "
      & integer'image(m'element'element'element'low) & " "
      & integer'image(m'element'element'element'high) & " "
      & integer'image(m'element'element'element'length) & " "
      & boolean'image(m'element'element'element'ascending) & " " & integer'image(n) & " "
      & integer'image(m'element'element'left) & " " & integer'image(m'element'left);
  end;
begin
  i0 : entity work.inner port map (p2 => s2, p3 => s3);

  process
    variable v2 : r2_t(0 to 2)(0 to 1)(15 downto 8);
    variable v3 : r3_t(0 to 1)(1 to 2)(0 to 3)(2 to 5);
    variable n : natural := 0;
  begin
    wait for 1 ns;
    report "var2 " & integer'image(v2'element'element'left) & " "
      & integer'image(v2'element'element'right) & " " & integer'image(v2'element'element'length);
    for i in v2'element'element'range loop n := n + 1; end loop;
    report "var2range " & integer'image(n);
    report "var3 " & integer'image(v3'element'element'element'left) & " "
      & integer'image(v3'element'element'element'right) & " "
      & integer'image(v3'element'element'element'length);
    report "sig2 " & integer'image(s2'element'element'left) & " "
      & integer'image(s2'element'element'high) & " " & integer'image(sa'element'element'left);
    report "sig3 " & integer'image(s3'element'element'element'left) & " "
      & integer'image(s3'element'element'element'length);
    report "p2s " & f2(s2);
    report "p2v " & f2(v2);
    report "p3s " & f3(s3);
    report "p3v " & f3(v3);
    report "const2 " & f2(k2) & " " & integer'image(k2'element'element'left);
    report "const3 " & f3(k3) & " " & integer'image(k3'element'element'element'left);
    wait;
  end process;
end;
