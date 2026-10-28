-- yavs-test: std=2008
-- expect: E1231 7:33 an element of a composite type cannot be of file type `ft`
-- expect: E1231 10:9 an element of a composite type cannot be of file type `ft`
-- LRM 5.3.1: an element of an array or record type is not of a file type (#813)
package p is
  type ft is file of integer;
  type arr is array (1 to 2) of ft;
  type rec is record
    a : integer;
    f : ft;
  end record;
end package;
