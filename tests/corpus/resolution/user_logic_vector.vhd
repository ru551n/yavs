-- difftest: top=user_logic_vector std=2008
-- Element-wise user resolution of std_ulogic_vector and integer array subtypes, driven
-- whole and by slices from several processes (#293).
library ieee;
use ieee.std_logic_1164.all;
entity user_logic_vector is end;
architecture t of user_logic_vector is
  function resolved_max (s : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := '0';
  begin
    for i in s'range loop
      if s(i) = '1' then r := '1'; elsif s(i) = 'X' and r = '0' then r := 'X'; end if;
    end loop;
    return r;
  end;
  subtype slv_max_c is (resolved_max) std_ulogic_vector;
  type int_vec is array (natural range <>) of integer;
  function maxres (v : int_vec) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop if v(i) > m then m := v(i); end if; end loop;
    return m;
  end;
  subtype rint is maxres integer;
  type rint_vec is array (natural range <>) of rint;
  signal v : slv_max_c(7 downto 0) := (others => '0');
  signal n : rint_vec(0 to 2) := (0, 0, 0);
begin
  v <= "10100000" after 1 ns, "00000000" after 4 ns;
  v(3 downto 0) <= "0110" after 2 ns;
  p1 : process begin
    v(7 downto 4) <= "0101";
    wait for 3 ns;
    v(7 downto 4) <= "X000";
    wait;
  end process;
  n <= (3, 1, 5) after 1 ns;
  n <= (2, 4, 0) after 1 ns;
  process (v) begin report "v=" & to_string(v) & " @" & time'image(now); end process;
  process (n) begin
    report "n=" & integer'image(n(0)) & "," & integer'image(n(1)) & "," & integer'image(n(2))
      & " @" & time'image(now);
  end process;
  process begin
    wait for 10 ns;
    assert v = "X0000110" and n = (3, 4, 5) severity failure;
    report "done";
    wait;
  end process;
end;
