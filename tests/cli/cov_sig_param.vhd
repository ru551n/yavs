-- Coverage of a procedure called with a signal actual narrower than its formal (its checked
-- body, #1181) and with one that is not: both calls count into the same items.
entity cov_sig_param is end entity;
architecture a of cov_sig_param is
  signal s : integer range 0 to 9 := 0;
  signal t : integer := 0;
  procedure drv (signal o : out integer; v : integer) is
  begin
    if v > 2 and v < 9 then
      o <= v;
    else
      o <= 1;
    end if;
  end procedure;
begin
  process
  begin
    drv(s, 5);
    drv(t, 3);
    drv(t, 0);
    wait;
  end process;
end architecture;
