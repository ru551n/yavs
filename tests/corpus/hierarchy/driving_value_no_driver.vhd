-- difftest: top=driving_value_no_driver std=2008 exit=1 ignore=fatalmsg,path
-- Evaluating 'driving_value of a signal in a process without a driver of it is an error (LRM 16.2.4). The message text is compared loosely (ignore=fatalmsg,path).
entity dv_leaf is port (q : out bit); end;
architecture a of dv_leaf is
begin
  q <= '1';
  process begin
    wait for 1 ns;
    report "reading" & bit'image(q'driving_value);
    wait;
  end process;
end;
entity driving_value_no_driver is end;
architecture t of driving_value_no_driver is
  signal s : bit;
begin
  u : entity work.dv_leaf port map (q => s);
end;
