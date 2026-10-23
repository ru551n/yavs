-- Signals that are not logic vectors: an integer counter, a bit_vector rotate and an enumeration:
-- zero-delay drives to byte signals.
entity reg_bytes is end;
architecture a of reg_bytes is
  type state is (idle, run, stop);
  signal clk : bit := '0';
  signal n : integer := 0;
  signal r : bit_vector(15 downto 0) := x"0001";
  signal s : state := idle;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if clk = '1' then
      n <= n + 1;
      r <= r(14 downto 0) & r(15);
      case s is
        when idle => s <= run;
        when run => s <= stop;
        when stop => s <= idle;
      end case;
    end if;
  end process;
  process begin
    wait for 1 us;
    report "n=" & integer'image(n) & " r=" & to_hstring(r) & " s=" & state'image(s);
    done <= true; wait;
  end process;
end;
