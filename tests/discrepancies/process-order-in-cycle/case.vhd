-- (from the differential corpus, process/process_order)
-- Order of the processes resumed in one cycle: the LRM leaves it open (14.7.5.3). yavs: at
-- initialisation elaboration order; then the processes with one suspension point in
-- elaboration order, then the others in reverse, signal-resumed before timeout-resumed (#252).
-- A concurrent procedure call whose procedure loops on one wait counts as several waits (#281).
-- Each process reports its position in the run order of the cycle.
package process_order_pkg is
  type counter is protected
    impure function take return integer;
  end protected;
end package;
package body process_order_pkg is
  type counter is protected body
    variable n : integer := 0;
    impure function take return integer is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
end package body;

use work.process_order_pkg.all;
package process_order_pos is
  shared variable pos : counter;
end package;

use work.process_order_pos.all;
entity process_order_leaf is port (go : in bit); end;
architecture a of process_order_leaf is
begin
  lm : process begin
    wait on go; report "RES u.lm two waits " & integer'image(pos.take); wait;
  end process;
  ls : process (go) begin
    if now > 0 ns then report "RES u.ls sensitivity list " & integer'image(pos.take); end if;
  end process;
end;

use work.process_order_pos.all;
entity process_order is end;
architecture t of process_order is
  signal go : bit;
  procedure watch (signal s : in bit) is
  begin
    loop
      wait on s; report "RES cc concurrent call " & integer'image(pos.take);
    end loop;
  end procedure;
begin
  go <= '1' after 1 ns;
  m1 : process begin
    wait on go; report "RES m1 event " & integer'image(pos.take); wait;
  end process;
  t1 : process begin
    wait for 1 ns; report "RES t1 timeout " & integer'image(pos.take); wait;
  end process;
  s1 : process (go) begin
    if now > 0 ns then report "RES s1 sensitivity list " & integer'image(pos.take); end if;
  end process;
  u : entity work.process_order_leaf port map (go);
  w1 : process begin
    wait on go; report "RES w1 one wait " & integer'image(pos.take);
  end process;
  m2 : process begin
    wait on go; report "RES m2 event " & integer'image(pos.take); wait;
  end process;
  t2 : process begin
    wait for 1 ns; report "RES t2 timeout " & integer'image(pos.take); wait;
  end process;
  pr : process
    procedure wait_go is begin wait on go; end procedure;
  begin
    wait_go; report "RES pr wait in a procedure " & integer'image(pos.take); wait;
  end process;
  cc : watch (go);
end;
