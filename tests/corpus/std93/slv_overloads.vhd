-- difftest: top=slv_overloads std=1993
-- In VHDL-93 STD_LOGIC_VECTOR is a type of its own: overloads on both vector types are legal.
library ieee;
use ieee.std_logic_1164.all;

package ovl is
  function img (v : std_logic_vector) return string;
  function img (v : std_ulogic_vector) return string;
  procedure check (name : string; a, b : std_logic_vector);
  procedure check (name : string; a, b : std_ulogic_vector);
end package;

package body ovl is
  function img (v : std_logic_vector) return string is
    variable s : string(1 to v'length);
    variable k : natural := 0;
  begin
    for i in v'range loop
      k := k + 1;
      s(k) := std_logic'image(v(i))(2);
    end loop;
    return "slv " & s;
  end;
  function img (v : std_ulogic_vector) return string is
    variable s : string(1 to v'length);
    variable k : natural := 0;
  begin
    for i in v'range loop
      k := k + 1;
      s(k) := std_ulogic'image(v(i))(2);
    end loop;
    return "sulv " & s;
  end;
  procedure check (name : string; a, b : std_logic_vector) is
  begin
    assert a = b report name & ": " & img(a) & " /= " & img(b) severity failure;
    report name & " ok " & img(a);
  end;
  procedure check (name : string; a, b : std_ulogic_vector) is
  begin
    assert a = b report name & ": " & img(a) & " /= " & img(b) severity failure;
    report name & " ok " & img(a);
  end;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.ovl.all;

entity slv_overloads is end;

architecture a of slv_overloads is
  signal bus_a : std_logic_vector(3 downto 0);
  signal u : std_ulogic_vector(3 downto 0) := "01XZ";
  signal tri : std_logic_vector(3 downto 0);
begin
  -- two drivers per element, resolved element by element
  tri <= "ZZ10";
  tri <= "Z1Z0";
  bus_a <= "1010";

  process
    variable sv : std_logic_vector(3 downto 0) := "01XZ";
  begin
    wait for 1 ns;
    check("slv", bus_a, "1010");
    check("sulv", u, "01XZ");
    check("convert up", std_ulogic_vector(sv), u);
    check("convert down", std_logic_vector(u), sv);
    report "tri " & img(tri);
    assert tri = "Z110" report "resolution" severity failure;
    check("and", bus_a and "0110", "0010");
    check("not", not bus_a, "0101");
    report "done";
    wait;
  end process;
end architecture;
