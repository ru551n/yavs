-- Eight clocked processes in one batch, each driving one integer register: one byte entry per
-- activation, moved by the batch loop with a store (`byte_moves` counts the moves of more).
entity byte_store is end;
architecture a of byte_store is
  type iv is array (0 to 7) of integer;
  signal clk : bit := '0';
  signal n : iv := (others => 0);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  g : for i in 0 to 7 generate
    signal c : integer := i;
  begin
    process (clk) begin
      if clk = '1' then c <= c + 1; end if;
    end process;
    n(i) <= c;
  end generate;
  process
    variable s : integer := 0;
  begin
    wait for 1 us;
    for i in 0 to 7 loop s := s + n(i) * (i + 1); end loop;
    report "s=" & integer'image(s);
    done <= true; wait;
  end process;
end;
