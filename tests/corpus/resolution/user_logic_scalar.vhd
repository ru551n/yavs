-- difftest: top=user_logic_scalar std=2008
-- User resolution functions on std_ulogic, bit and boolean scalars (#293).
library ieee;
use ieee.std_logic_1164.all;
entity user_logic_scalar is end;
architecture t of user_logic_scalar is
  -- '1' wins, then 'H', then '0', else 'Z'.
  function resolved_max (s : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := 'Z';
  begin
    for i in s'range loop
      if s(i) = '1' then return '1'; end if;
      if s(i) = 'H' then r := 'H'; elsif s(i) = '0' and r = 'Z' then r := '0'; end if;
    end loop;
    return r;
  end;
  -- Counts the drivers: '0' none, '1' one, 'X' more, 'W' when one of them is 'W'.
  function count_res (s : std_ulogic_vector) return std_ulogic is
  begin
    for i in s'range loop
      if s(i) = 'W' then return 'W'; end if;
    end loop;
    case s'length is
      when 0 => return '0';
      when 1 => return '1';
      when others => return 'X';
    end case;
  end;
  type bit_vec is array (natural range <>) of bit;
  function bit_or (s : bit_vec) return bit is
  begin
    for i in s'range loop if s(i) = '1' then return '1'; end if; end loop;
    return '0';
  end;
  type bool_vec is array (natural range <>) of boolean;
  function all_true (s : bool_vec) return boolean is
  begin
    for i in s'range loop if not s(i) then return false; end if; end loop;
    return true;
  end;
  subtype std_logic_max is resolved_max std_ulogic;
  subtype counted is count_res std_ulogic;
  subtype rbit is bit_or bit;
  subtype rbool is all_true boolean;
  signal x : std_logic_max := '0';
  signal c : counted;
  signal b : rbit;
  signal t : rbool := true;
begin
  x <= '1' after 1 ns, '0' after 5 ns;
  x <= 'U';
  x <= 'Z', 'H' after 2 ns, '0' after 3 ns;
  c <= 'L', 'W' after 4 ns;
  c <= '-';
  c <= 'U';
  b <= '0', '1' after 2 ns, '0' after 3 ns;
  b <= '0', '1' after 3 ns, '0' after 4500 ps;
  t <= true, false after 2500 ps, true after 3500 ps;
  t <= true;
  process (x) begin report "x=" & std_ulogic'image(x) & " @" & time'image(now); end process;
  process (c) begin report "c=" & std_ulogic'image(c) & " @" & time'image(now); end process;
  process (b) begin report "b=" & bit'image(b) & " @" & time'image(now); end process;
  process (t) begin report "t=" & boolean'image(t) & " @" & time'image(now); end process;
  process begin
    wait for 10 ns;
    assert x = '0' and c = 'W' and b = '0' and t severity failure;
    report "done";
    wait;
  end process;
end;
