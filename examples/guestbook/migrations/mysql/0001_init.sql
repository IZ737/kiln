-- 0001 init
-- Written by `kiln db diff` for mysql. Read it before applying it.

create table accounts (
    id bigint auto_increment primary key,
    email text not null,
    name text not null,
    password text not null
);

create table entries (
    id bigint auto_increment primary key,
    account_id bigint not null,
    message text not null,
    foreign key (account_id) references accounts(id)
);

