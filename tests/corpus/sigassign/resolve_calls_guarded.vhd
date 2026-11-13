-- difftest: top=resolve_calls_guarded
-- A resolution function that reports each call, on a guarded integer bus with three guarded
-- drivers whose guards go on and off: one call per update with the connected sources (LRM
-- 14.7.3.2), a driver that disconnects (a null transaction) included, not one call per driver.
-- (No report for no source: the references differ on calls at initialisation.)
library ieee; use ieee.std_logic_1164.all;
package resolve_calls_guarded_pkg is
  type int_v is array (natural range <>) of integer;
  function res_max (v : int_v) return integer;
  subtype rint is res_max integer;
end;
package body resolve_calls_guarded_pkg is
  function res_max (v : int_v) return integer is
    variable m : integer := 0;
  begin
    for i in v'range loop if v(i) > m then m := v(i); end if; end loop;
    if m /= 0 then
      report "res_max of " & integer'image(v'length) & " sources: " & integer'image(m);
    end if;
    return m;
  end;
end;
library ieee; use ieee.std_logic_1164.all; use work.resolve_calls_guarded_pkg.all;
entity resolve_calls_guarded is end;
architecture a of resolve_calls_guarded is
  type int_arr is array (0 to 2) of integer;
  signal src : int_arr := (0, 0, 0);
  signal en : bit_vector(0 to 2) := "000";
  signal b : rint bus := 0;
begin
  process begin
    for i in 1 to 5 loop
      src <= (i * 10, i * 10 + 1, i * 10 + 2);
      case i is
        when 1 => en <= "111";
        when 2 => en <= "101";
        when 3 => en <= "001";
        when 4 => en <= "000";
        when others => en <= "110";
      end case;
      wait for 10 ns;
    end loop;
    wait;
  end process;
  g : for k in 0 to 2 generate
    blk : block (en(k) = '1') begin
      b <= guarded src(k);
    end block;
  end generate;
  process (b) begin report "b=" & integer'image(b); end process;
end;
