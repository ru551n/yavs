-- difftest: top=user_logic_record std=2008
-- A record of std_ulogic and std_ulogic_vector elements with user resolution functions,
-- each field driven by several processes (#293).
library ieee;
use ieee.std_logic_1164.all;
entity user_logic_record is end;
architecture t of user_logic_record is
  function resolved_max (s : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := '0';
  begin
    for i in s'range loop
      if s(i) = '1' then r := '1'; end if;
    end loop;
    return r;
  end;
  subtype std_logic_max is resolved_max std_ulogic;
  subtype slv_max_c is (resolved_max) std_ulogic_vector;
  type rec is record
    valid : std_logic_max;
    data  : slv_max_c(3 downto 0);
    count : integer;
  end record;
  signal r : rec := ('0', "0000", 0);
begin
  a : process begin
    r.valid <= '0';
    r.data <= "0001";
    wait for 1 ns;
    r.valid <= '1';
    r.data <= "0011";
    wait for 2 ns;
    r.valid <= '0';
    wait;
  end process;
  b : process begin
    r.data <= "0000";
    wait for 2 ns;
    r.valid <= '1';
    r.data <= "1000";
    wait for 2 ns;
    r.valid <= '0';
    wait;
  end process;
  r.count <= 5 after 1 ns;
  process (r) begin
    report "valid=" & std_ulogic'image(r.valid) & " data=" & to_string(r.data)
      & " count=" & integer'image(r.count) & " @" & time'image(now);
  end process;
  process begin
    wait for 10 ns;
    assert r.valid = '0' and r.data = "1011" and r.count = 5 severity failure;
    report "done";
    wait;
  end process;
end;
