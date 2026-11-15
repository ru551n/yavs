-- Byte lanes written from a call's result: the slice's descriptor, made before the value, keeps
-- its length across the call, so the store needs no length check and copies a constant count.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity slice_call_len is end;
architecture t of slice_call_len is
begin
  process
    variable v : std_ulogic_vector(63 downto 0);
    variable acc : natural := 0;
  begin
    for n in 0 to 39 loop
      for i in 0 to 7 loop
        v(8 * i + 7 downto 8 * i) := std_ulogic_vector(to_signed((n + i) mod 100, 8));
      end loop;
      if v(0) = '1' then
        acc := acc + 1;
      end if;
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
