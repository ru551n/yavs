entity alpha is
end entity beta;

architecture rtl of alpha is
begin
  l1 : process begin wait; end process l2;
end architecture;
