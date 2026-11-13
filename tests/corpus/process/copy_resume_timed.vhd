-- difftest: top=copy_resume_timed stop=15002ns
-- Generate lanes waiting in `wait on m(i) until m(i) = target for T` with a lane's own target and
-- timeout: most wakes leave the condition false, so the wait waits again with the deadline of its
-- statement and usually ends by the timeout. A process switches to its specialised copy between
-- two activations (after 1024 of them by default); the copy must keep the queued timeout and its
-- deadline and see this lane's element. Each lane runs about 1500 activations, past the switch.
entity copy_resume_timed is
  generic (N : positive := 3);
end entity;
architecture t of copy_resume_timed is
  type nat_array_t is array (natural range <>) of natural;
  signal clk  : bit := '0';
  signal done : boolean := false;
  signal cnt  : natural := 0;
  signal m    : nat_array_t(0 to N - 1) := (others => 0);
  signal hits, outs, sums : nat_array_t(0 to N - 1) := (others => 0);
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if clk = '1' then cnt <= cnt + 1; end if;
  end process;
  g : for i in 0 to N - 1 generate
    m(i) <= (3 * cnt + i) mod 50;
    process
      constant target : natural := (7 * i) mod 50;
      constant tmo : time := 35 ns + i * 1 ns;
      variable start : time;
      variable h, o, s : natural := 0;
    begin
      start := now;
      wait on m(i) until m(i) = target for tmo;
      if m(i) = target and m(i)'event then
        h := h + 1;
      else
        assert now - start = tmo report "lane " & integer'image(i) & " resumed early at "
          & time'image(now) severity failure;
        o := o + 1;
      end if;
      s := (s + (now - start) / 1 ns) mod 100000;
      hits(i) <= h;
      outs(i) <= o;
      sums(i) <= s;
    end process;
  end generate;
  process begin
    wait for 15 us;
    done <= true;
    wait for 1 ns;
    for i in 0 to N - 1 loop
      report "lane " & integer'image(i) & ": " & integer'image(hits(i)) & " "
        & integer'image(outs(i)) & " " & integer'image(sums(i));
    end loop;
    wait;
  end process;
end architecture;
