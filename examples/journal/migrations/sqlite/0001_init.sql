-- 0001 init
-- Written by `kiln db diff`. Read it before applying it.

create table comments (
    id integer primary key,
    post_id integer not null references posts(id),
    user_id integer not null references users(id),
    body text not null
);

create table posts (
    id integer primary key,
    user_id integer not null references users(id),
    title text not null,
    body text not null,
    published integer not null
);

create table users (
    id integer primary key,
    email text not null,
    name text not null,
    password text not null,
    role text not null
);

