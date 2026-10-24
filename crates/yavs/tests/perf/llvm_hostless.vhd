-- Compiled code reaches instance data (a table that depends on a generic) and the variables of
-- the process a nested procedure belongs to in place, without a call into the host per access.
entity llvm_hostless is
  generic (g : integer := 3);
end;
architecture a of llvm_hostless is
  type tab_t is array (0 to 15) of integer;
  function make(k : integer) return tab_t is
    variable t : tab_t;
  begin
    for i in t'range loop t(i) := k * i + 1; end loop;
    return t;
  end;
  constant tab : tab_t := make(g);
  signal clk : bit := '0';
  signal acc : integer := 0;
begin
  clk <= not clk after 5 ns when now < 400 ns;
  process (clk)
    variable v : integer := 0;
    procedure bump(k : integer) is
    begin
      if k mod 2 = 1 then v := v + k; end if;
      v := v mod 1000003;
    end;
  begin
    if clk = '1' then
      for i in tab'range loop
        bump(tab(i) + i);
      end loop;
      acc <= v;
    end if;
  end process;
  process
  begin
    wait for 450 ns;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
