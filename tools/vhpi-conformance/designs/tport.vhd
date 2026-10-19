-- An integer output port driven by a transport delay from an input port without a default,
-- and a counter, as written to and watched through VHPI (#347).
entity tport is
  port (i_int : in integer; o_int : out integer);
end entity;

architecture sim of tport is
  signal s_int : integer := 0;
begin
  process begin
    wait for 10 ns;
    s_int <= s_int + 1;
  end process;

  o_int <= transport i_int after 10 ns;
end architecture;
