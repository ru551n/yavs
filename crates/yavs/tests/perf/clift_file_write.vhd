-- A process writes an integer to a typed file at every clock edge: compiled code runs the
-- file's `write` itself (`direct_native`), not through a step of the interpreter.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity clift_file_write is end;
architecture a of clift_file_write is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(7 downto 0) := x"01";
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then
      q <= q(6 downto 0) & (q(7) xor q(5));
    end if;
  end process;
  process
    type file_t is file of integer;
    file f : file_t;
    variable n : integer := 0;
  begin
    file_open(f, "clift_file_write.bin", write_mode);
    for i in 1 to 100 loop
      wait until rising_edge(clk);
      write(f, to_integer(unsigned(q)));
      n := n + to_integer(unsigned(q));
    end loop;
    file_close(f);
    report "n=" & integer'image(n);
    done <= true;
    wait;
  end process;
end;
