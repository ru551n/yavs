-- Branch arm names in coverage reports (#877): if/elsif without else, if/else, case
-- with a choice list and others, conditional and selected assignments, a loop.
entity cov_report is
end entity;

architecture a of cov_report is
  signal en, rst : bit := '0';
  signal n       : natural := 0;
  signal sel     : natural range 0 to 3 := 0;
  signal y, zc, zs : bit;
begin
  stim : process
  begin
    wait for 1 ns;
    en <= '1';
    wait for 1 ns;
    sel <= 1;
    wait for 1 ns;
    en <= '0';
    wait;
  end process;

  count : process (en, rst)
  begin
    if rst = '1' then
      n <= 0;
    elsif en = '1' and n < 15 then
      n <= n + 1;
    elsif n > 100 then
      n <= 0;
    end if;
  end process;

  pick : process (sel)
    variable v : natural := 0;
  begin
    case sel is
      when 0 => v := 1;
      when 1 | 2 => v := 2;
      when others => v := 3;
    end case;
    if v < 10 then
      v := 1;
    else
      v := 0;
    end if;
    for i in 1 to 2 loop
      v := v + i;
    end loop;
  end process;

  y <= '1' when en = '1' else '0';
  zc <= '1' when sel < 4;
  with sel select
    zs <= '1' when 0 | 1,
         '0' when others;
end architecture;
