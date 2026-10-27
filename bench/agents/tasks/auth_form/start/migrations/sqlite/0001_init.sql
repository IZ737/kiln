-- 0001 init
-- Written by `kiln db diff` for sqlite. Read it before applying it.

create table accounts (
    id integer primary key,
    email text not null,
    name text not null,
    password text not null
);

