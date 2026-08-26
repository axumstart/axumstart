import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';

const Libraries = [
  {
    title: 'axumstart_components',
    to: '/docs/components/',
    description:
      'An async dependency-injection container: derive macros for registering components, ' +
      'an OnCreate lifecycle hook, and an Inject<T> extractor for Axum handlers.',
  },
  {
    title: 'axumstart_db',
    to: '/docs/db/',
    description:
      'A #[repository] macro that turns trait method names like find_by_user_id or ' +
      'insert_all into sqlx queries against Postgres, MySQL, or SQLite.',
  },
];

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary')}>
      <div className="container">
        <Heading as="h1" className="hero__title">
          {siteConfig.title}
        </Heading>
        <p className="hero__subtitle">{siteConfig.tagline}</p>
        <div className="margin-top--md">
          <Link className="button button--secondary button--lg" to="/docs/">
            Get Started
          </Link>
        </div>
      </div>
    </header>
  );
}

function LibraryCards() {
  return (
    <section className="container margin-vert--xl">
      <div className="row">
        {Libraries.map(({title, to, description}) => (
          <div key={title} className="col col--6">
            <div className="card margin-bottom--lg">
              <div className="card__header">
                <Heading as="h3">{title}</Heading>
              </div>
              <div className="card__body">
                <p>{description}</p>
              </div>
              <div className="card__footer">
                <Link className="button button--primary button--block" to={to}>
                  Read the docs
                </Link>
              </div>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

export default function Home() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout title={siteConfig.title} description={siteConfig.tagline}>
      <HomepageHeader />
      <main>
        <LibraryCards />
      </main>
    </Layout>
  );
}
