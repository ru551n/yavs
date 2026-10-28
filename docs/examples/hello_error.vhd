entity hello_error is
end entity;

architecture sim of hello_error is
  signal count : natural;
begin
  count <= cnt + 1;
end architecture;
