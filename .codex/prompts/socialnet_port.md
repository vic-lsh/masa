# Porting microservice from the socialnetwork application

You're a senior software engineer experienced in porting microservice applications from C++ Apache Thrift to Rust gRPC Tonic.

You're porting one of the services in the thrift file below:

(Thrift file definition copied from the DeathStarBench repo's socialnet application)
```
namespace cpp social_network
namespace py social_network
namespace lua social_network

struct User {
    1: i64 user_id;
    2: string first_name;
    3: string last_name;
    4: string username;
    5: string password_hashed;
    6: string salt;
}

enum ErrorCode {
  SE_CONNPOOL_TIMEOUT,
  SE_THRIFT_CONN_ERROR,
  SE_UNAUTHORIZED,
  SE_MEMCACHED_ERROR,
  SE_MONGODB_ERROR,
  SE_REDIS_ERROR,
  SE_THRIFT_HANDLER_ERROR,
  SE_RABBITMQ_CONN_ERROR
}

exception ServiceException {
    1: ErrorCode errorCode;
    2: string message;
}

enum PostType {
  POST,
  REPOST,
  REPLY,
  DM
}

struct Media {
  1: i64 media_id;
  2: string media_type;
}

struct Url {
  1: string shortened_url;
  2: string expanded_url;
}

struct UserMention {
  1: i64 user_id;
  2: string username;
}

struct Creator {
  1: i64 user_id;
  2: string username;
}

struct TextServiceReturn {
 1: string text;
 2: list<UserMention> user_mentions;
 3: list<Url> urls;
}

struct Post {
  1: i64 post_id;
  2: Creator creator;
  3: i64 req_id;
  4: string text;
  5: list<UserMention> user_mentions;
  6: list<Media> media;
  7: list<Url> urls;
  8: i64 timestamp;
  9: PostType post_type;
}

service UniqueIdService {
  i64 ComposeUniqueId (
      1: i64 req_id,
      2: PostType post_type,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service TextService {
  TextServiceReturn ComposeText (
      1: i64 req_id,
      2: string text,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service UserService {
  void RegisterUser (
      1: i64 req_id,
      2: string first_name,
      3: string last_name,
      4: string username,
      5: string password,
      6: map<string, string> carrier
  ) throws (1: ServiceException se)

  void RegisterUserWithId (
      1: i64 req_id,
      2: string first_name,
      3: string last_name,
      4: string username,
      5: string password,
      6: i64 user_id,
      7: map<string, string> carrier
  ) throws (1: ServiceException se)

  string Login(
      1: i64 req_id,
      2: string username,
      3: string password,
      4: map<string, string> carrier
  ) throws (1: ServiceException se)

  Creator ComposeCreatorWithUserId(
      1: i64 req_id,
      2: i64 user_id,
      3: string username,
      4: map<string, string> carrier
  ) throws (1: ServiceException se)

  Creator ComposeCreatorWithUsername(
      1: i64 req_id,
      2: string username,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)

  i64 GetUserId(
      1: i64 req_id,
      2: string username,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service ComposePostService {
  void ComposePost(
    1: i64 req_id,
    2: string username,
    3: i64 user_id,
    4: string text,
    5: list<i64> media_ids,
    6: list<string> media_types,
    7: PostType post_type,
    8: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service PostStorageService {
  void StorePost(
    1: i64 req_id,
    2: Post post,
    3: map<string, string> carrier
  ) throws (1: ServiceException se)

  Post ReadPost(
    1: i64 req_id,
    2: i64 post_id,
    3: map<string, string> carrier
  ) throws (1: ServiceException se)

  list<Post> ReadPosts(
    1: i64 req_id,
    2: list<i64> post_ids,
    3: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service HomeTimelineService {
  list<Post> ReadHomeTimeline(
    1: i64 req_id,
    2: i64 user_id,
    3: i32 start,
    4: i32 stop,
    5: map<string, string> carrier
  ) throws (1: ServiceException se)

  void WriteHomeTimeline(
    1: i64 req_id,
    2: i64 post_id,
    3: i64 user_id,
    4: i64 timestamp,
    5: list<i64> user_mentions_id,
    6: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service UserTimelineService {
  void WriteUserTimeline(
    1: i64 req_id,
    2: i64 post_id,
    3: i64 user_id,
    4: i64 timestamp,
    5: map<string, string> carrier
  ) throws (1: ServiceException se)

  list<Post> ReadUserTimeline(
    1: i64 req_id,
    2: i64 user_id,
    3: i32 start,
    4: i32 stop,
    5: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service SocialGraphService{
  list<i64> GetFollowers(
      1: i64 req_id,
      2: i64 user_id,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)

  list<i64> GetFollowees(
      1: i64 req_id,
      2: i64 user_id,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)

  void Follow(
      1: i64 req_id,
      2: i64 user_id,
      3: i64 followee_id,
      4: map<string, string> carrier
  ) throws (1: ServiceException se)

  void Unfollow(
      1: i64 req_id,
      2: i64 user_id,
      3: i64 followee_id,
      4: map<string, string> carrier
  ) throws (1: ServiceException se)

  void FollowWithUsername(
      1: i64 req_id,
      2: string user_usernmae,
      3: string followee_username,
      4: map<string, string> carrier
  ) throws (1: ServiceException se)

  void UnfollowWithUsername(
      1: i64 req_id,
      2: string user_usernmae,
      3: string followee_username,
      4: map<string, string> carrier
  ) throws (1: ServiceException se)

  void InsertUser(
      1: i64 req_id,
      2: i64 user_id,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service UserMentionService {
  list<UserMention> ComposeUserMentions(
      1: i64 req_id,
      2: list<string> usernames,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service UrlShortenService {
  list<Url> ComposeUrls(
      1: i64 req_id,
      2: list<string> urls,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)

  list<string> GetExtendedUrls(
      1: i64 req_id,
      2: list<string> shortened_urls,
      3: map<string, string> carrier
  ) throws (1: ServiceException se)
}

service MediaService {
  list<Media> ComposeMedia(
      1: i64 req_id,
      2: list<string> media_types,
      3: list<i64> media_ids,
      4: map<string, string> carrier
  ) throws (1: ServiceException se)
}
```

Your role is to take a look at the user's provided C++ microservice and port it to a corresponding Rust implementation.

## Implementation location

You will need to write a .proto file and write a implementation.

### Proto file

Please add one at

```
apps/socialnet/proto/<service>.proto
```

Name <service> such that it does not contain the word service. For example,
UserTimelineService is named `user_timeline.proto`, and CompostPostService
is named `compost_post.proto`.

### Implementation

Please implement each new service in `apps/socialnet/src`. Make a new directory for each service.

Each service should follow the following structure:

```
apps/socialnet/src/<service_name>/main.rs # entry point; minimal logic
apps/socialnet/src/<service_name>/server.rs # core server implementation
```

## Example

You may look at other directories under `apps/socialnet/src/` for reference implementations for a service.

## Dependencies

If you need to introduce new packages, please do so under `apps/socialnet/Cargo.toml`.

When picking a package (especially database related once), first check if the package
is already included. If not, then prefer installing the async version of this package
(e.g., `redis-async` over `redis`).
If you failed to find an async version of a package, report so to me at the end.

## Configuration values

If the implementation depends on configuration values such as a database connection string,
put the required configs in an `Args` struct. Add documentation on each field to explain
what the field means.

## What you're NOT allowed to edit

You are not allowed to edit beyond apps/socialnet. Ideally, you should not need to modify
beyond apps/socialnet/<service_name>, where <service_name> is the new service you're implementing.

## Final note

Do not stop until your new binary compiles.

The implementation to port follows below:
