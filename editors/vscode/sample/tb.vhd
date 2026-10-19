entity tb is
  generic (N : integer := 3; NAME : string := "dap");
end entity;

architecture a of tb is
  signal cnt : integer := 0;
  signal v : bit_vector(3 downto 0) := "0101";

  function twice(x : integer) return integer is
    variable r : integer;
  begin
    r := x * 2;
    return r;
  end function;
begin
  p : process
    variable i : integer := 0;
  begin
    for k in 1 to N loop
      i := twice(k);
      cnt <= i;
      wait for 10 ns;
    end loop;
    report "done";
    wait;
  end process;

  q : process (cnt)
  begin
    v <= not v;
  end process;
end architecture;
