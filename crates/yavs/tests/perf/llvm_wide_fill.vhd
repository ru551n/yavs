-- Functions on native frames that fill (LFill) and compare (LEq) more than 64 logic elements: the
-- instruction is stepped on a frame of its own, the frame is not materialised (#648).
library ieee; use ieee.std_logic_1164.all;
entity llvm_wide_fill is end;
architecture a of llvm_wide_fill is
  type slm is array (natural range <>, natural range <>) of std_logic;
  type slvv is array (natural range <>) of std_logic_vector(7 downto 0);
  function get_row(m : slm; r : natural) return std_logic_vector is
    variable v : std_logic_vector(7 downto 0);
  begin
    for i in v'range loop v(i) := m(r, i); end loop;
    return v;
  end function;
  function rows(m : slm) return slvv is
    variable res : slvv(m'range(1));
  begin
    for i in m'range(1) loop res(i) := get_row(m, i); end loop;
    return res;
  end function;
  function same(m : slm; r : natural) return boolean is
    variable a, b : std_logic_vector(127 downto 0);
  begin
    for i in a'range loop a(i) := m(r, i mod 8); b(i) := m(r + 1, i mod 8); end loop;
    return a = b;
  end function;
begin
  process
    variable m : slm(0 to 127, 7 downto 0) := (others => (others => '0'));
    variable r : slvv(0 to 127);
    variable acc : natural := 0;
  begin
    for k in 0 to 199 loop
      m(k mod 128, k mod 8) := '1';
      r := rows(m);
      if same(m, k mod 100) then acc := acc + 1; end if;
      for i in r'range loop
        if r(i)(k mod 8) = '1' then acc := acc + 1; end if;
      end loop;
    end loop;
    report "fills=" & integer'image(acc);
    wait;
  end process;
end;
