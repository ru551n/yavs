-- Copy and fill loops over logic vectors in functions that run on native frames under --tier=llvm:
-- each is planned at its head, its iterations but the last done at once (the short row copies
-- inline, the fills by a runtime call, cb_eloop), and the rest stays in the compiled code (no
-- step, no deopt, no lookup call).
library ieee;
use ieee.std_logic_1164.all;
entity llvm_eloop is end;
architecture a of llvm_eloop is
  type slm is array (natural range <>, natural range <>) of std_logic;
  function row(m : slm; k : natural) return std_logic_vector is
    variable r : std_logic_vector(m'high(2) downto m'low(2));
  begin
    for i in r'range loop r(i) := m(k, i); end loop;
    return r;
  end function;
  function ones(n : natural) return std_logic_vector is
    variable r : std_logic_vector(n - 1 downto 0) := (others => '0');
  begin
    for i in r'range loop r(i) := '1'; end loop;
    return r;
  end function;
begin
  process
    variable m : slm(0 to 7, 7 downto 0);
    variable v : std_logic_vector(7 downto 0);
    variable w : std_logic_vector(30 downto 0);
    variable acc : integer := 0;
  begin
    for k in 0 to 7 loop
      for j in 7 downto 0 loop
        if (j + k) mod 3 = 0 then m(k, j) := '1'; else m(k, j) := '0'; end if;
      end loop;
    end loop;
    for rep in 1 to 20 loop
      -- (A pure function with repeated arguments is answered from the call memo.)
      if m(rep mod 8, rep mod 8) = '1' then m(rep mod 8, rep mod 8) := '0'; else m(rep mod 8, rep mod 8) := '1'; end if;
      for k in 0 to 7 loop
        v := row(m, k);
        for i in v'range loop
          if v(i) = '1' then acc := (acc + i * rep + k) mod 100003; end if;
        end loop;
      end loop;
      w(8 + rep - 1 downto 0) := ones(8 + rep);
      for i in 8 + rep - 1 downto 0 loop
        if w(i) = '1' then acc := acc + 1; end if;
      end loop;
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
